use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use futures::StreamExt;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use unicode_width::UnicodeWidthStr;

use crate::daemon::client::{self, DaemonClientError, DaemonPingStatus};
use crate::daemon::events::DaemonEvent;
use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::{DaemonInfo, WorkerRow};
use crate::daemon::supervisor::WorkerState;

#[cfg(not(test))]
const LOG_FOLLOW_POLL_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(test)]
const LOG_FOLLOW_POLL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(not(test))]
const EVENT_FOLLOW_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const EVENT_FOLLOW_CONNECT_TIMEOUT: Duration = Duration::from_millis(200);
#[cfg(not(test))]
const EVENT_FOLLOW_HEADER_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const EVENT_FOLLOW_HEADER_TIMEOUT: Duration = Duration::from_millis(200);
const API_PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b'/')
    .add(b'?')
    .add(b'#')
    .add(b'[')
    .add(b']')
    .add(b'@')
    .add(b':');

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Ps {
        json: bool,
    },
    Cron {
        command: CronCommand,
        json: bool,
    },
    Projects {
        command: ProjectsCommand,
        json: bool,
    },
    Restart {
        target: Option<String>,
        daemon: bool,
        json: bool,
    },
    Stop {
        target: Option<String>,
        daemon: bool,
        json: bool,
    },
    Logs {
        target: Option<String>,
        daemon: bool,
        follow: bool,
        json: bool,
    },
    Events {
        kind: Option<String>,
        follow: bool,
        json: bool,
    },
    Status {
        json: bool,
    },
    Doctor {
        json: bool,
    },
    Version {
        json: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectsCommand {
    List,
    Open { path: PathBuf },
    Pin { target: String },
    Unpin { target: String },
    Forget { target: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronCommand {
    List { project: Option<String> },
    Add(CronAddOptions),
    Run { project: Option<String>, id: String },
    Remove { project: Option<String>, id: String },
    Pause { project: Option<String>, id: String },
    Resume { project: Option<String>, id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronAddOptions {
    pub project: Option<String>,
    pub cron: Option<String>,
    pub every: Option<String>,
    pub at: Option<String>,
    pub tz: Option<String>,
    pub hook_id: Option<String>,
    pub prompt: Option<String>,
    pub command: Option<String>,
    pub command_argv: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub timeout_secs: Option<u64>,
    pub delivery: CronDeliveryArg,
    pub recurring: Option<bool>,
    pub durable: bool,
    pub isolated: bool,
    pub description: String,
    pub chat_id: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronDeliveryArg {
    Chat,
    Webhook {
        url: String,
        token: Option<String>,
    },
    Notifier {
        integration_id: String,
        target: Option<String>,
    },
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOptions {
    pub command: CliCommand,
}

impl CliOptions {
    fn json_output(&self) -> bool {
        match &self.command {
            CliCommand::Ps { json }
            | CliCommand::Cron { json, .. }
            | CliCommand::Projects { json, .. }
            | CliCommand::Restart { json, .. }
            | CliCommand::Stop { json, .. }
            | CliCommand::Logs { json, .. }
            | CliCommand::Events { json, .. }
            | CliCommand::Status { json }
            | CliCommand::Doctor { json }
            | CliCommand::Version { json } => *json,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub pid: u32,
    pub version: String,
    pub port: u16,
    pub started_at_ms: u64,
    pub uptime_secs: u64,
    pub workers: u64,
    pub cron_pending: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Serialize)]
struct PsOutput {
    daemon: DaemonStatus,
    workers: Vec<WorkerRow>,
}

#[derive(Debug, Serialize)]
struct ProjectsOutput {
    projects: Vec<ProjectEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn exit_code(&self) -> i32 {
        if self.checks.iter().all(|check| check.ok) {
            0
        } else {
            1
        }
    }
}

#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub exit_code: i32,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: format!("error: {}\n\n{}", message.into(), usage_text()),
            exit_code: 2,
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
        }
    }
}

pub fn parse_cli_args(args: &[std::ffi::OsString]) -> Result<CliOptions, String> {
    let mut args = args
        .iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if args.is_empty() {
        return Err(usage_text().to_string());
    }
    let subcommand = args.remove(0);
    let options = match subcommand.as_str() {
        "ps" => CliOptions {
            command: CliCommand::Ps {
                json: take_flag(&mut args, "--json")?,
            },
        },
        "cron" => parse_cron(&mut args)?,
        "projects" => parse_projects(&mut args)?,
        "restart" => parse_restart_stop(&mut args, true)?,
        "stop" => parse_restart_stop(&mut args, false)?,
        "logs" => parse_logs(&mut args)?,
        "events" => parse_events(&mut args)?,
        "status" => CliOptions {
            command: CliCommand::Status {
                json: take_flag(&mut args, "--json")?,
            },
        },
        "doctor" => CliOptions {
            command: CliCommand::Doctor {
                json: take_flag(&mut args, "--json")?,
            },
        },
        "version" => CliOptions {
            command: CliCommand::Version {
                json: take_flag(&mut args, "--json")?,
            },
        },
        _ => {
            return Err(format!(
                "unknown subcommand `{subcommand}`\n\n{}",
                usage_text()
            ))
        }
    };
    if !args.is_empty() {
        return Err(format!(
            "unexpected argument `{}`\n\n{}",
            args[0],
            usage_text()
        ));
    }
    Ok(options)
}

fn parse_cron(args: &mut Vec<String>) -> Result<CliOptions, String> {
    let json = take_flag(args, "--json")?;
    let command_name = take_one(args, "cron command")?;
    let command = match command_name.as_str() {
        "list" => CronCommand::List {
            project: take_option(args, "--project")?,
        },
        "add" => CronCommand::Add(parse_cron_add(args)?),
        "run" => CronCommand::Run {
            project: take_option(args, "--project")?,
            id: take_one(args, "id")?,
        },
        "rm" | "remove" => CronCommand::Remove {
            project: take_option(args, "--project")?,
            id: take_one(args, "id")?,
        },
        "pause" => CronCommand::Pause {
            project: take_option(args, "--project")?,
            id: take_one(args, "id")?,
        },
        "resume" => CronCommand::Resume {
            project: take_option(args, "--project")?,
            id: take_one(args, "id")?,
        },
        other => {
            return Err(format!(
                "unknown cron command `{other}`\n\n{}",
                usage_text()
            ))
        }
    };
    Ok(CliOptions {
        command: CliCommand::Cron { command, json },
    })
}

fn parse_cron_add(args: &mut Vec<String>) -> Result<CronAddOptions, String> {
    let project = take_option(args, "--project")?;
    let cron = take_option(args, "--cron")?;
    let every = take_option(args, "--every")?;
    let at = take_option(args, "--at")?;
    let tz = take_option(args, "--tz")?;
    let hook_id = take_option(args, "--hook-id")?;
    let prompt = take_option(args, "--prompt")?;
    let command = take_option(args, "--command")?;
    let command_argv = take_option(args, "--command-argv")?
        .map(|value| {
            serde_json::from_str::<Vec<String>>(&value)
                .map_err(|error| format!("--command-argv requires a JSON string array: {error}"))
        })
        .transpose()?;
    let cwd = take_option(args, "--cwd")?;
    let timeout_secs = take_option(args, "--timeout-secs")?
        .map(|value| parse_positive_u64(&value, "--timeout-secs"))
        .transpose()?;
    let delivery = parse_cron_delivery(args)?;
    let recurring = take_optional_bool(args, "--recurring")?;
    let durable = take_flag(args, "--durable")?;
    let isolated = take_flag(args, "--isolated")?;
    let chat_id = take_option(args, "--chat-id")?;
    let mode = take_option(args, "--mode")?;
    let description = match take_option(args, "--description")? {
        Some(description) => description,
        None => take_one(args, "--description")?,
    };
    if description.trim().is_empty() {
        return Err("--description must not be empty".to_string());
    }
    if description.chars().count() > 80 {
        return Err("--description must be at most 80 characters".to_string());
    }
    if cron.is_none() && every.is_none() && at.is_none() && hook_id.is_none() {
        return Err("one of --cron, --every, --at, or --hook-id is required".to_string());
    }
    let action_count = usize::from(prompt.is_some())
        + usize::from(command.is_some())
        + usize::from(command_argv.is_some());
    if action_count != 1 {
        return Err(
            "exactly one of --prompt, --command, or --command-argv is required".to_string(),
        );
    }
    Ok(CronAddOptions {
        project,
        cron,
        every,
        at,
        tz,
        hook_id,
        prompt,
        command,
        command_argv,
        cwd,
        timeout_secs,
        delivery,
        recurring,
        durable,
        isolated,
        description: description.trim().to_string(),
        chat_id,
        mode,
    })
}

fn parse_cron_delivery(args: &mut Vec<String>) -> Result<CronDeliveryArg, String> {
    let delivery = take_option(args, "--delivery")?.unwrap_or_else(|| "chat".to_string());
    match delivery.as_str() {
        "chat" => Ok(CronDeliveryArg::Chat),
        "none" => Ok(CronDeliveryArg::None),
        "webhook" => Ok(CronDeliveryArg::Webhook {
            url: take_option(args, "--webhook-url")?
                .ok_or_else(|| "--delivery webhook requires --webhook-url".to_string())?,
            token: take_option(args, "--webhook-token")?,
        }),
        "notifier" => Ok(CronDeliveryArg::Notifier {
            integration_id: take_option(args, "--notifier")?
                .ok_or_else(|| "--delivery notifier requires --notifier".to_string())?,
            target: take_option(args, "--notifier-target")?,
        }),
        other => Err(format!(
            "--delivery must be chat, webhook, notifier, or none; got `{other}`"
        )),
    }
}

fn take_optional_bool(args: &mut Vec<String>, flag: &str) -> Result<Option<bool>, String> {
    take_option(args, flag)?
        .map(|value| parse_bool(&value, flag))
        .transpose()
}

fn parse_bool(value: &str, flag: &str) -> Result<bool, String> {
    match value {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(format!("{flag} requires true or false")),
    }
}

fn parse_positive_u64(value: &str, flag: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{flag} requires a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{flag} requires a positive integer"));
    }
    Ok(parsed)
}

fn parse_projects(args: &mut Vec<String>) -> Result<CliOptions, String> {
    let json = take_flag(args, "--json")?;
    let command = if args.is_empty() {
        ProjectsCommand::List
    } else {
        match args.remove(0).as_str() {
            "open" => ProjectsCommand::Open {
                path: PathBuf::from(take_one(args, "path")?),
            },
            "pin" => ProjectsCommand::Pin {
                target: take_one(args, "id|path")?,
            },
            "unpin" => ProjectsCommand::Unpin {
                target: take_one(args, "id|path")?,
            },
            "forget" => ProjectsCommand::Forget {
                target: take_one(args, "id|path")?,
            },
            other => {
                return Err(format!(
                    "unknown projects command `{other}`\n\n{}",
                    usage_text()
                ))
            }
        }
    };
    Ok(CliOptions {
        command: CliCommand::Projects { command, json },
    })
}

fn parse_restart_stop(args: &mut Vec<String>, restart: bool) -> Result<CliOptions, String> {
    let json = take_flag(args, "--json")?;
    let daemon = take_flag(args, "--daemon")?;
    let target = if daemon {
        None
    } else {
        Some(take_one(args, "id|path")?)
    };
    let command = if restart {
        CliCommand::Restart {
            target,
            daemon,
            json,
        }
    } else {
        CliCommand::Stop {
            target,
            daemon,
            json,
        }
    };
    Ok(CliOptions { command })
}

fn parse_logs(args: &mut Vec<String>) -> Result<CliOptions, String> {
    let json = take_flag(args, "--json")?;
    let follow_short = take_flag(args, "-f")?;
    let follow_long = take_flag(args, "--follow")?;
    let follow = follow_short || follow_long;
    let daemon = take_flag(args, "--daemon")?;
    let target = if args.is_empty() {
        None
    } else {
        Some(args.remove(0))
    };
    if json && follow {
        return Err("logs --json is incompatible with -f/--follow".to_string());
    }
    if daemon && target.is_some() {
        return Err("logs --daemon does not accept a project target".to_string());
    }
    Ok(CliOptions {
        command: CliCommand::Logs {
            target,
            daemon,
            follow,
            json,
        },
    })
}

fn parse_events(args: &mut Vec<String>) -> Result<CliOptions, String> {
    let json = take_flag(args, "--json")?;
    let follow_short = take_flag(args, "-f")?;
    let follow_long = take_flag(args, "--follow")?;
    let follow = follow_short || follow_long;
    let kind = take_option(args, "--kind")?;
    if json && follow {
        return Err("events --json is incompatible with -f/--follow".to_string());
    }
    Ok(CliOptions {
        command: CliCommand::Events { kind, follow, json },
    })
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> Result<bool, String> {
    let mut found = false;
    args.retain(|arg| {
        if arg == flag {
            found = true;
            false
        } else {
            true
        }
    });
    Ok(found)
}

fn take_option(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    if let Some(index) = args.iter().position(|arg| arg == flag) {
        args.remove(index);
        if index >= args.len() {
            return Err(format!("missing value for {flag}"));
        }
        return Ok(Some(args.remove(index)));
    }
    Ok(None)
}

fn take_one(args: &mut Vec<String>, name: &str) -> Result<String, String> {
    if args.is_empty() {
        Err(format!("missing {name}"))
    } else {
        Ok(args.remove(0))
    }
}

pub async fn run(options: CliOptions) -> i32 {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    run_with_io(options, &mut stdout, &mut stderr).await
}

pub async fn run_with_io(
    options: CliOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let json_output = options.json_output();
    match run_inner(options, stdout).await {
        Ok(code) => code,
        Err(error) => {
            if json_output {
                let _ = print_json(
                    stdout,
                    &json!({"ok": false, "error": error.message, "exit_code": error.exit_code}),
                );
            } else {
                let _ = writeln!(stderr, "{}", error.message);
            }
            error.exit_code
        }
    }
}

async fn run_inner(options: CliOptions, out: &mut dyn Write) -> Result<i32, CliError> {
    match options.command {
        CliCommand::Doctor { json } => {
            let report = doctor_report().await;
            if json {
                print_json(out, &report)?;
            } else {
                write_doctor(out, &report)?;
            }
            Ok(report.exit_code())
        }
        CliCommand::Version { json } => run_version(json, out).await,
        CliCommand::Restart {
            daemon: true, json, ..
        } => restart_daemon(json, out).await,
        CliCommand::Stop {
            daemon: true, json, ..
        } => stop_daemon(json, out).await,
        CliCommand::Ps { json } => {
            let daemon = ensure_daemon().await?;
            let status: DaemonStatus = client_get(&daemon, "/daemon/v1/status").await?;
            let workers: Vec<WorkerRow> = client_get(&daemon, "/daemon/v1/workers").await?;
            if json {
                print_json(
                    out,
                    &PsOutput {
                        daemon: status,
                        workers,
                    },
                )?;
            } else {
                write_ps(out, &status, &workers)?;
            }
            Ok(0)
        }
        CliCommand::Projects { command, json } => run_projects(command, json, out).await,
        CliCommand::Cron { command, json } => run_cron(command, json, out).await,
        CliCommand::Restart { target, json, .. } => {
            let daemon = existing_daemon().await?;
            let projects = list_projects(&daemon).await?;
            let id = resolve_target(&projects, target.as_deref().unwrap_or_default())?;
            let worker: Value =
                client_post_empty(&daemon, &format!("/daemon/v1/projects/{id}/restart")).await?;
            print_value(out, json, &worker, "restarted")?;
            Ok(0)
        }
        CliCommand::Stop { target, json, .. } => {
            let daemon = existing_daemon().await?;
            let projects = list_projects(&daemon).await?;
            let id = resolve_target(&projects, target.as_deref().unwrap_or_default())?;
            let worker: Value =
                client_post_empty(&daemon, &format!("/daemon/v1/projects/{id}/stop")).await?;
            print_value(out, json, &worker, "stopped")?;
            Ok(0)
        }
        CliCommand::Logs {
            target,
            daemon: daemon_logs,
            follow,
            json,
        } => run_logs(target, daemon_logs, follow, json, out).await,
        CliCommand::Events { kind, follow, json } => run_events(kind, follow, json, out).await,
        CliCommand::Status { json } => run_status(json, out).await,
    }
}

async fn run_projects(
    command: ProjectsCommand,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let daemon = ensure_daemon().await?;
    match command {
        ProjectsCommand::List => {
            let projects = list_projects(&daemon).await?;
            if json_output {
                print_json(out, &ProjectsOutput { projects })?;
            } else {
                write_projects(out, &projects)?;
            }
        }
        ProjectsCommand::Open { path } => {
            let root = canonicalize_existing_dir(&path)?;
            let value: Value =
                client::post_json(&daemon, "/daemon/v1/projects/open", &json!({"root": root}))
                    .await
                    .map_err(client_error)?;
            print_value(out, json_output, &value, "opened")?;
        }
        ProjectsCommand::Pin { target } => {
            set_project_pin(&daemon, &target, true, json_output, out).await?;
        }
        ProjectsCommand::Unpin { target } => {
            set_project_pin(&daemon, &target, false, json_output, out).await?;
        }
        ProjectsCommand::Forget { target } => {
            let projects = list_projects(&daemon).await?;
            let id = resolve_target(&projects, &target)?;
            let value: Value = client::delete_json(&daemon, &format!("/daemon/v1/projects/{id}"))
                .await
                .map_err(client_error)?;
            print_value(out, json_output, &value, "forgotten")?;
        }
    }
    Ok(0)
}

async fn run_cron(
    command: CronCommand,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let daemon = ensure_daemon().await?;
    match command {
        CronCommand::List { project } => {
            let project_id = resolve_or_open_project(&daemon, project.as_deref()).await?;
            let tasks: Value = client_get(
                &daemon,
                &cron_project_path(&project_id, "/v1/scheduler/cron"),
            )
            .await?;
            if json_output {
                print_json(out, &tasks)?;
            } else {
                write_cron_list(out, tasks.as_array().map(Vec::as_slice).unwrap_or(&[]))?;
            }
        }
        CronCommand::Add(options) => {
            let project_id = resolve_or_open_project(&daemon, options.project.as_deref()).await?;
            let request = cron_add_request(options);
            let value: Value = client::post_json(
                &daemon,
                &cron_project_path(&project_id, "/v1/scheduler/cron"),
                &request,
            )
            .await
            .map_err(client_error)?;
            print_value(out, json_output, &value, "created")?;
        }
        CronCommand::Run { project, id } => {
            let project_id = resolve_or_open_project(&daemon, project.as_deref()).await?;
            let value: Value = client_post_empty(
                &daemon,
                &cron_project_path(
                    &project_id,
                    &format!("/v1/scheduler/cron/{}/run", path_segment(&id)),
                ),
            )
            .await?;
            print_value(out, json_output, &value, "triggered")?;
        }
        CronCommand::Remove { project, id } => {
            let project_id = resolve_or_open_project(&daemon, project.as_deref()).await?;
            let value: Value = client::delete_json(
                &daemon,
                &cron_project_path(
                    &project_id,
                    &format!("/v1/scheduler/cron/{}", path_segment(&id)),
                ),
            )
            .await
            .map_err(client_error)?;
            print_value(out, json_output, &value, "removed")?;
        }
        CronCommand::Pause { project, id } => {
            patch_cron_enabled(&daemon, project.as_deref(), &id, false, json_output, out).await?;
        }
        CronCommand::Resume { project, id } => {
            patch_cron_enabled(&daemon, project.as_deref(), &id, true, json_output, out).await?;
        }
    }
    Ok(0)
}

async fn patch_cron_enabled(
    daemon: &DaemonInfo,
    project: Option<&str>,
    id: &str,
    enabled: bool,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let project_id = resolve_or_open_project(daemon, project).await?;
    let value: Value = client::patch_json(
        daemon,
        &cron_project_path(
            &project_id,
            &format!("/v1/scheduler/cron/{}", path_segment(id)),
        ),
        &json!({"enabled": enabled}),
    )
    .await
    .map_err(client_error)?;
    print_value(
        out,
        json_output,
        &value,
        if enabled { "resumed" } else { "paused" },
    )
}

async fn resolve_or_open_project(
    daemon: &DaemonInfo,
    project: Option<&str>,
) -> Result<String, CliError> {
    match project {
        Some(target) => resolve_target(&list_projects(daemon).await?, target),
        None => {
            let root = canonicalize_existing_dir(Path::new("."))?;
            let value: Value =
                client::post_json(daemon, "/daemon/v1/projects/open", &json!({"root": root}))
                    .await
                    .map_err(client_error)?;
            value
                .get("project_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| CliError::runtime("daemon project open response missing project_id"))
        }
    }
}

fn cron_project_path(project_id: &str, suffix: &str) -> String {
    format!("/p/{}{}", path_segment(project_id), suffix)
}

fn path_segment(value: &str) -> String {
    utf8_percent_encode(value, API_PATH_SEGMENT).to_string()
}

fn cron_add_request(options: CronAddOptions) -> Value {
    let mut request = serde_json::Map::new();
    insert_string(&mut request, "cron", options.cron);
    insert_string(&mut request, "every", options.every);
    insert_string(&mut request, "at", options.at);
    insert_string(&mut request, "tz", options.tz);
    insert_string(&mut request, "hook_id", options.hook_id);
    insert_string(&mut request, "prompt", options.prompt);
    insert_string(&mut request, "command", options.command);
    if let Some(argv) = options.command_argv {
        request.insert("command_argv".to_string(), json!(argv));
    }
    insert_string(&mut request, "cwd", options.cwd);
    if let Some(timeout_secs) = options.timeout_secs {
        request.insert("timeout_secs".to_string(), json!(timeout_secs));
    }
    request.insert(
        "delivery".to_string(),
        cron_delivery_value(options.delivery),
    );
    if let Some(recurring) = options.recurring {
        request.insert("recurring".to_string(), json!(recurring));
    }
    if options.durable {
        request.insert("durable".to_string(), json!(true));
    }
    request.insert("isolated".to_string(), json!(options.isolated));
    request.insert("description".to_string(), json!(options.description));
    insert_string(&mut request, "chat_id", options.chat_id);
    insert_string(&mut request, "mode", options.mode);
    Value::Object(request)
}

fn insert_string(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        map.insert(key.to_string(), json!(value));
    }
}

fn cron_delivery_value(delivery: CronDeliveryArg) -> Value {
    match delivery {
        CronDeliveryArg::Chat => json!("chat"),
        CronDeliveryArg::None => json!("none"),
        CronDeliveryArg::Webhook { url, token } => {
            let mut value = json!({"kind": "webhook", "url": url});
            if let Some(token) = token {
                value["token"] = json!(token);
            }
            value
        }
        CronDeliveryArg::Notifier {
            integration_id,
            target,
        } => {
            let mut value = json!({"kind": "notifier", "integration_id": integration_id});
            if let Some(target) = target {
                value["target"] = json!(target);
            }
            value
        }
    }
}

fn write_cron_list(out: &mut dyn Write, tasks: &[Value]) -> Result<(), CliError> {
    if tasks.is_empty() {
        writeln!(out, "No scheduled jobs").map_err(write_error)?;
        return Ok(());
    }
    writeln!(out, "ID\tENABLED\tSCHEDULE\tNEXT\tDESCRIPTION").map_err(write_error)?;
    for task in tasks {
        let id = task.get("id").and_then(Value::as_str).unwrap_or("");
        let enabled = task
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let schedule = task
            .get("human_schedule")
            .or_else(|| task.get("cron"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let next = task
            .get("next_fire_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let description = task
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        writeln!(out, "{id}\t{enabled}\t{schedule}\t{next}\t{description}").map_err(write_error)?;
    }
    Ok(())
}

async fn set_project_pin(
    daemon: &DaemonInfo,
    target: &str,
    pinned: bool,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let projects = list_projects(daemon).await?;
    let id = resolve_target(&projects, target)?;
    let value: Value = client::post_json(
        daemon,
        &format!("/daemon/v1/projects/{id}/pin"),
        &json!({"pinned": pinned}),
    )
    .await
    .map_err(client_error)?;
    print_value(
        out,
        json_output,
        &value,
        if pinned { "pinned" } else { "unpinned" },
    )?;
    Ok(())
}

async fn run_logs(
    target: Option<String>,
    daemon_logs: bool,
    follow: bool,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let daemon = ensure_daemon().await?;
    let (path, file_path) = log_source(&daemon, target, daemon_logs).await?;
    let follow_state = if follow && !json_output {
        Some(initial_log_follow_state(&file_path).await)
    } else {
        None
    };
    let text = client::get_text(&daemon, &path)
        .await
        .map_err(client_error)?;
    if json_output {
        print_json(out, &json!({"log": text}))?;
    } else {
        write!(out, "{text}").map_err(write_error)?;
        if let Some(follow_state) = follow_state {
            follow_logs(&file_path, follow_state, out).await?;
        }
    }
    Ok(0)
}

async fn log_source(
    daemon: &DaemonInfo,
    target: Option<String>,
    daemon_logs: bool,
) -> Result<(String, PathBuf), CliError> {
    if daemon_logs || target.is_none() {
        return Ok((
            "/daemon/v1/logs?tail=200".to_string(),
            crate::daemon::paths::daemon_log_path(),
        ));
    }
    let projects = list_projects(daemon).await?;
    let id = resolve_target(&projects, target.as_deref().unwrap_or_default())?;
    let slug = projects
        .iter()
        .find(|project| project.id == id)
        .map(|project| project.slug.clone())
        .ok_or_else(|| CliError::runtime(format!("project not registered: {id}")))?;
    Ok((
        format!("/daemon/v1/logs?project_id={id}&tail=200"),
        crate::daemon::paths::logs_dir().join(format!("worker-{slug}.log")),
    ))
}

const LOG_HEAD_FINGERPRINT_LEN: u64 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogFollowState {
    offset: u64,
    identity: Option<LogFileIdentity>,
    head: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LogFileIdentity {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

async fn follow_logs(
    path: &Path,
    mut state: LogFollowState,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = tokio::time::sleep(LOG_FOLLOW_POLL_INTERVAL) => {
                let delta = read_log_delta(path, &mut state).await?;
                if !delta.is_empty() {
                    write!(out, "{delta}").map_err(write_error)?;
                }
            }
        }
    }
}

async fn initial_log_follow_state(path: &Path) -> LogFollowState {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => LogFollowState {
            offset: metadata.len(),
            identity: Some(log_file_identity(&metadata)),
            head: read_log_head(path).await,
        },
        Err(_) => LogFollowState {
            offset: 0,
            identity: None,
            head: None,
        },
    }
}

async fn read_log_head(path: &Path) -> Option<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let file = tokio::fs::File::open(path).await.ok()?;
    let mut buf = Vec::new();
    file.take(LOG_HEAD_FINGERPRINT_LEN)
        .read_to_end(&mut buf)
        .await
        .ok()?;
    Some(buf)
}

async fn read_log_delta(path: &Path, state: &mut LogFollowState) -> Result<String, CliError> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            state.offset = 0;
            state.identity = None;
            state.head = None;
            return Ok(String::new());
        }
        Err(error) => {
            return Err(CliError::runtime(format!(
                "failed to open log file {}: {error}",
                path.display()
            )))
        }
    };
    let metadata = file
        .metadata()
        .await
        .map_err(|error| CliError::runtime(format!("failed to stat log file: {error}")))?;
    let identity = log_file_identity(&metadata);
    let len = metadata.len();
    let head = read_log_head(path).await;
    let head_changed = match (&state.head, &head) {
        (Some(prev), Some(cur)) => {
            let n = prev.len().min(cur.len());
            prev[..n] != cur[..n]
        }
        _ => false,
    };
    if state.identity != Some(identity) || head_changed {
        state.offset = 0;
    } else if state.offset > len {
        state.offset = 0;
    }
    state.identity = Some(identity);
    state.head = head;
    file.seek(std::io::SeekFrom::Start(state.offset))
        .await
        .map_err(|error| CliError::runtime(format!("failed to seek log file: {error}")))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .await
        .map_err(|error| CliError::runtime(format!("failed to read log file: {error}")))?;
    let valid_len = valid_utf8_prefix_len(&bytes);
    if valid_len == 0 {
        return Ok(String::new());
    }
    let text = std::str::from_utf8(&bytes[..valid_len])
        .map_err(|error| CliError::runtime(format!("invalid log UTF-8 boundary: {error}")))?
        .to_string();
    state.offset = state.offset.saturating_add(valid_len as u64);
    Ok(text)
}

#[cfg(unix)]
fn log_file_identity(metadata: &std::fs::Metadata) -> LogFileIdentity {
    use std::os::unix::fs::MetadataExt;

    LogFileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn log_file_identity(_: &std::fs::Metadata) -> LogFileIdentity {
    LogFileIdentity {}
}

fn valid_utf8_prefix_len(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(error) => error.valid_up_to(),
    }
}

async fn run_events(
    kind: Option<String>,
    follow: bool,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let daemon = ensure_daemon().await?;
    if follow {
        follow_events(&daemon, kind.as_deref(), json_output, out).await?;
        return Ok(0);
    }
    let text = client::get_text(&daemon, "/daemon/v1/events")
        .await
        .map_err(client_error)?;
    let events = parse_sse_events(&text)
        .into_iter()
        .filter(|event| {
            kind.as_ref()
                .map(|kind| &event.kind == kind)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    if json_output {
        print_json(out, &events)?;
    } else {
        for event in events {
            write_event(out, &event, false)?;
        }
    }
    Ok(0)
}

async fn follow_events(
    daemon: &DaemonInfo,
    kind: Option<&str>,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let url = format!(
        "{}/daemon/v1/events?follow=true",
        crate::daemon::chat_client::daemon_base_url(daemon)
    );
    let request = match &daemon.auth_token {
        Some(token) => event_follow_client().get(url).bearer_auth(token),
        None => event_follow_client().get(url),
    };
    let response = tokio::time::timeout(EVENT_FOLLOW_HEADER_TIMEOUT, request.send())
        .await
        .map_err(|_| {
            CliError::runtime("daemon request failed: timed out waiting for event stream headers")
        })?
        .map_err(|error| {
            CliError::runtime(format!(
                "daemon request failed: failed to contact daemon: {error}"
            ))
        })?;
    if !response.status().is_success() {
        return Err(CliError::runtime(format!(
            "daemon request failed with status {}",
            response.status()
        )));
    }
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            next = stream.next() => {
                let Some(chunk) = next else { return Ok(()); };
                let chunk = chunk.map_err(|error| CliError::runtime(format!("daemon event stream failed: {error}")))?;
                buffer.extend_from_slice(&chunk);
                for block in drain_complete_sse_frames(&mut buffer)? {
                    for event in parse_sse_events(&(block + "\n\n")) {
                        if kind.map(|kind| event.kind == kind).unwrap_or(true) {
                            write_event(out, &event, json_output)?;
                        }
                    }
                }
            }
        }
    }
}

fn event_follow_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(EVENT_FOLLOW_CONNECT_TIMEOUT)
            .build()
            .expect("failed to build daemon event follow client")
    })
}

async fn run_status(json_output: bool, out: &mut dyn Write) -> Result<i32, CliError> {
    match client::read_daemon_json().await {
        Ok(None) => {
            if json_output {
                print_json(
                    out,
                    &json!({"reachable": false, "reason": "daemon.json not found"}),
                )?;
            } else {
                writeln!(out, "daemon not running: daemon.json not found").map_err(write_error)?;
            }
            Ok(1)
        }
        Ok(Some(info)) => match client_get::<DaemonStatus>(&info, "/daemon/v1/status").await {
            Ok(status) => {
                if json_output {
                    print_json(out, &json!({"reachable": true, "status": status}))?;
                } else {
                    writeln!(
                        out,
                        "daemon healthy: pid {}, port {}, version {}, uptime {}s, workers {}",
                        status.pid, status.port, status.version, status.uptime_secs, status.workers
                    )
                    .map_err(write_error)?;
                }
                Ok(0)
            }
            Err(error) => {
                if json_output {
                    print_json(out, &json!({"reachable": false, "reason": error.message}))?;
                } else {
                    writeln!(out, "daemon not reachable: {}", error.message)
                        .map_err(write_error)?;
                }
                Ok(1)
            }
        },
        Err(error) => Err(CliError::runtime(format!(
            "failed to read daemon.json: {error}"
        ))),
    }
}

async fn run_version(json_output: bool, out: &mut dyn Write) -> Result<i32, CliError> {
    let daemon = match client::read_daemon_json().await {
        Ok(Some(info)) if client::ping_daemon(&info).await.is_alive() => Some(info.version),
        Ok(_) => None,
        Err(error) => return Err(CliError::runtime(error.to_string())),
    };
    if json_output {
        print_json(
            out,
            &json!({"client": env!("CARGO_PKG_VERSION"), "daemon": daemon}),
        )?;
    } else {
        writeln!(out, "{}", crate::cli_dispatch::version_text()).map_err(write_error)?;
        match daemon {
            Some(version) => {
                writeln!(out, "{:>20} {}", "daemon_version", version).map_err(write_error)?
            }
            None => writeln!(out, "{:>20} unreachable", "daemon_version").map_err(write_error)?,
        }
    }
    Ok(0)
}

async fn restart_daemon(json_output: bool, out: &mut dyn Write) -> Result<i32, CliError> {
    if let Some(info) = client::read_daemon_json()
        .await
        .map_err(|error| CliError::runtime(error.to_string()))?
    {
        match client::ping_daemon(&info).await {
            DaemonPingStatus::Alive => {
                client::shutdown_daemon(&info, "restart")
                    .await
                    .map_err(CliError::runtime)?;
                client::wait_for_daemon_stop(&info, Duration::from_secs(15))
                    .await
                    .map_err(CliError::runtime)?;
            }
            DaemonPingStatus::NotRunning { .. } => {}
            DaemonPingStatus::Error { message } => {
                return Err(CliError::runtime(format!(
                    "daemon reachable but unhealthy: {message}"
                )));
            }
        }
    }
    let daemon = ensure_daemon().await?;
    if json_output {
        print_json(out, &daemon)?;
    } else {
        writeln!(
            out,
            "daemon restarted: pid {}, port {}",
            daemon.pid, daemon.port
        )
        .map_err(write_error)?;
    }
    Ok(0)
}

async fn stop_daemon(json_output: bool, out: &mut dyn Write) -> Result<i32, CliError> {
    let Some(info) = client::read_daemon_json()
        .await
        .map_err(|error| CliError::runtime(error.to_string()))?
    else {
        return print_daemon_not_running(json_output, out, "missing");
    };
    match client::ping_daemon(&info).await {
        DaemonPingStatus::Alive => {}
        DaemonPingStatus::NotRunning { .. } => {
            return print_daemon_not_running(json_output, out, "stale");
        }
        DaemonPingStatus::Error { message } => {
            return Err(CliError::runtime(format!(
                "daemon reachable but unhealthy: {message}"
            )));
        }
    }
    client::shutdown_daemon(&info, "stop")
        .await
        .map_err(CliError::runtime)?;
    client::wait_for_daemon_stop(&info, Duration::from_secs(15))
        .await
        .map_err(CliError::runtime)?;
    if json_output {
        print_json(out, &json!({"stopped": true}))?;
    } else {
        writeln!(out, "daemon stopped").map_err(write_error)?;
    }
    Ok(0)
}

fn print_daemon_not_running(
    json_output: bool,
    out: &mut dyn Write,
    reason: &str,
) -> Result<i32, CliError> {
    if json_output {
        print_json(out, &json!({"stopped": false, "reason": reason}))?;
    } else {
        writeln!(out, "no daemon running ({reason})").map_err(write_error)?;
    }
    Ok(0)
}

async fn doctor_report() -> DoctorReport {
    let daemon_json_path = crate::daemon::paths::daemon_json_path();
    let mut checks = Vec::new();
    checks.push(binary_path_check());
    let info = match crate::daemon::state::read_daemon_info(&daemon_json_path).await {
        Ok(Some(info)) => {
            checks.push(check(
                "daemon.json",
                true,
                format!("valid: {}", daemon_json_path.display()),
            ));
            Some(info)
        }
        Ok(None) => {
            checks.push(check(
                "daemon.json",
                false,
                format!("missing: {}", daemon_json_path.display()),
            ));
            None
        }
        Err(error) => {
            checks.push(check("daemon.json", false, error));
            None
        }
    };
    if let Some(info) = &info {
        let status = client::get_json::<DaemonStatus>(info, "/daemon/v1/status").await;
        let reachable = status.is_ok();
        checks.push(check(
            "daemon reachable",
            reachable,
            if reachable {
                format!("port {}", info.port)
            } else {
                format!("unreachable at port {}", info.port)
            },
        ));
        let version_ok = info.version == env!("CARGO_PKG_VERSION");
        checks.push(check(
            "version match",
            version_ok,
            format!(
                "client {}, daemon {}",
                env!("CARGO_PKG_VERSION"),
                info.version
            ),
        ));
        let port_ok = std::net::TcpStream::connect(("127.0.0.1", info.port)).is_ok();
        checks.push(check(
            "loopback port",
            port_ok,
            format!("127.0.0.1:{}", info.port),
        ));
        checks.push(check(
            "advertised urls",
            !info.urls.loopback.is_empty(),
            info.urls.loopback.clone(),
        ));
        if reachable {
            match client::get_json::<Vec<WorkerRow>>(info, "/daemon/v1/workers").await {
                Ok(workers) => {
                    let status_workers = status.as_ref().map(|status| status.workers).unwrap_or(0);
                    let active_workers = workers
                        .iter()
                        .filter(|row| !matches!(row.state, WorkerState::Stopped))
                        .count() as u64;
                    let responsive = workers_responsive(&workers).await;
                    let missing = workers
                        .iter()
                        .filter(|row| !row.root.exists())
                        .map(|row| row.slug.clone())
                        .collect::<Vec<_>>();
                    checks.push(check(
                        "workers responsive",
                        responsive,
                        format!("{} active workers", active_workers),
                    ));
                    checks.push(check(
                        "worker count",
                        status_workers == active_workers,
                        format!("status {status_workers}, active listed {active_workers}"),
                    ));
                    checks.push(check(
                        "project roots",
                        missing.is_empty(),
                        if missing.is_empty() {
                            "all present".to_string()
                        } else {
                            format!("missing: {}", missing.join(", "))
                        },
                    ));
                }
                Err(error) => {
                    checks.push(check("workers responsive", false, error.to_string()));
                    checks.push(check("worker count", false, error.to_string()));
                }
            }
        } else {
            checks.push(check("workers responsive", false, "daemon unreachable"));
            checks.push(check("worker count", false, "daemon unreachable"));
        }
    } else {
        checks.push(check("daemon reachable", false, "daemon.json unavailable"));
        checks.push(check("version match", false, "daemon.json unavailable"));
        checks.push(check("loopback port", false, "daemon.json unavailable"));
        checks.push(check("advertised urls", false, "daemon.json unavailable"));
        checks.push(check(
            "workers responsive",
            false,
            "daemon.json unavailable",
        ));
        checks.push(check("worker count", false, "daemon.json unavailable"));
    }
    checks.push(check(
        "lock file",
        crate::daemon::paths::lock_path().exists(),
        crate::daemon::paths::lock_path().display().to_string(),
    ));
    DoctorReport { checks }
}

fn binary_path_check() -> DoctorCheck {
    match std::env::current_exe() {
        Ok(path) => check(
            "binary path",
            path.is_file(),
            if path.is_file() {
                path.display().to_string()
            } else {
                format!("not a file: {}", path.display())
            },
        ),
        Err(error) => check("binary path", false, error.to_string()),
    }
}

async fn workers_responsive(workers: &[WorkerRow]) -> bool {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(200))
        .timeout(Duration::from_millis(500))
        .build();
    let Ok(client) = client else {
        return false;
    };
    for worker in workers {
        if !matches!(worker.state, WorkerState::Ready) {
            continue;
        }
        let Some(port) = worker.http_port else {
            return false;
        };
        let ok = match client
            .get(format!("http://127.0.0.1:{port}/v1/ping"))
            .send()
            .await
        {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

fn check(name: impl Into<String>, ok: bool, message: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.into(),
        ok,
        message: message.into(),
    }
}

async fn ensure_daemon() -> Result<DaemonInfo, CliError> {
    client::ensure_daemon_running()
        .await
        .map_err(|error| CliError::runtime(format!("daemon unavailable after auto-start: {error}")))
}

async fn existing_daemon() -> Result<DaemonInfo, CliError> {
    let Some(info) = client::read_daemon_json()
        .await
        .map_err(|error| CliError::runtime(error.to_string()))?
    else {
        return Err(CliError::runtime(
            "daemon not running: daemon.json not found",
        ));
    };
    match client::ping_daemon(&info).await {
        DaemonPingStatus::Alive => Ok(info),
        DaemonPingStatus::NotRunning { .. } => {
            Err(CliError::runtime("daemon not running: stale daemon.json"))
        }
        DaemonPingStatus::Error { message } => Err(CliError::runtime(format!(
            "daemon reachable but unhealthy: {message}"
        ))),
    }
}

async fn client_get<T: for<'de> Deserialize<'de>>(
    daemon: &DaemonInfo,
    path: &str,
) -> Result<T, CliError> {
    client::get_json(daemon, path).await.map_err(client_error)
}

async fn client_post_empty<T: for<'de> Deserialize<'de>>(
    daemon: &DaemonInfo,
    path: &str,
) -> Result<T, CliError> {
    client::post_empty_json(daemon, path)
        .await
        .map_err(client_error)
}

fn client_error(error: DaemonClientError) -> CliError {
    CliError::runtime(format!("daemon request failed: {error}"))
}

async fn list_projects(daemon: &DaemonInfo) -> Result<Vec<ProjectEntry>, CliError> {
    client_get(daemon, "/daemon/v1/projects").await
}

fn resolve_target(projects: &[ProjectEntry], target: &str) -> Result<String, CliError> {
    if target.is_empty() {
        return Err(CliError::usage("missing project id or path"));
    }
    let matches = projects
        .iter()
        .filter(|project| project.id.starts_with(target))
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        return Ok(matches[0].id.clone());
    }
    if matches.len() > 1 {
        return Err(CliError::runtime(format!(
            "ambiguous project id prefix `{target}`"
        )));
    }
    let root = canonicalize_existing_dir(Path::new(target))?;
    if let Some(project) = projects.iter().find(|project| project.root == root) {
        Ok(project.id.clone())
    } else {
        Err(CliError::runtime(format!(
            "project not registered: {}",
            root.display()
        )))
    }
}

fn canonicalize_existing_dir(path: &Path) -> Result<PathBuf, CliError> {
    let root = crate::files_correction::canonical_path(path.to_string_lossy().to_string());
    if !root.exists() {
        return Err(CliError::runtime(format!(
            "path does not exist: {}",
            path.display()
        )));
    }
    if !root.is_dir() {
        return Err(CliError::runtime(format!(
            "path is not a directory: {}",
            root.display()
        )));
    }
    Ok(root)
}

pub fn format_worker_table(rows: &[WorkerRow]) -> String {
    let headers = [
        "PROJECT",
        "STATE",
        "PID",
        "CLIENTS",
        "BUSY",
        "HTTP",
        "LSP",
        "CRON-NEXT",
        "IDLE-IN",
        "PIN",
    ];
    let mut table = vec![headers.iter().map(|s| s.to_string()).collect::<Vec<_>>()];
    for row in rows {
        table.push(vec![
            format!(
                "{}+{}",
                row.slug,
                row.project_id.chars().take(8).collect::<String>()
            ),
            format!("{:?}", row.state).to_lowercase(),
            row.pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            row.lsp_clients.to_string(),
            row.busy_chats.to_string(),
            row.http_port
                .map(|port| port.to_string())
                .unwrap_or_else(|| "-".to_string()),
            row.lsp_port
                .map(|port| port.to_string())
                .unwrap_or_else(|| "-".to_string()),
            row.cron_next_fire_ms
                .map(|ms| ms.to_string())
                .unwrap_or_else(|| "-".to_string()),
            row.idle_deadline_ms
                .map(|ms| ms.to_string())
                .unwrap_or_else(|| "-".to_string()),
            if row.pinned {
                "yes".to_string()
            } else {
                "no".to_string()
            },
        ]);
    }
    format_table(&table)
}

fn format_projects_table(projects: &[ProjectEntry]) -> String {
    let mut table = vec![vec![
        "ID".to_string(),
        "SLUG".to_string(),
        "PIN".to_string(),
        "ROOT".to_string(),
    ]];
    for project in projects {
        table.push(vec![
            project.id.clone(),
            project.slug.clone(),
            if project.pinned {
                "yes".to_string()
            } else {
                "no".to_string()
            },
            project.root.display().to_string(),
        ]);
    }
    format_table(&table)
}

fn format_table(rows: &[Vec<String>]) -> String {
    let mut widths = Vec::new();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if widths.len() <= index {
                widths.push(0);
            }
            widths[index] = widths[index].max(display_width(cell));
        }
    }
    let mut out = String::new();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                out.push_str("  ");
            }
            out.push_str(cell);
            if index + 1 < row.len() {
                out.push_str(&" ".repeat(widths[index].saturating_sub(display_width(cell))));
            }
        }
        out.push('\n');
    }
    out
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

fn write_ps(
    out: &mut dyn Write,
    status: &DaemonStatus,
    rows: &[WorkerRow],
) -> Result<(), CliError> {
    writeln!(
        out,
        "daemon pid={} port={} version={} uptime={}s",
        status.pid, status.port, status.version, status.uptime_secs
    )
    .map_err(write_error)?;
    write!(out, "{}", format_worker_table(rows)).map_err(write_error)
}

fn write_projects(out: &mut dyn Write, projects: &[ProjectEntry]) -> Result<(), CliError> {
    write!(out, "{}", format_projects_table(projects)).map_err(write_error)
}

fn write_doctor(out: &mut dyn Write, report: &DoctorReport) -> Result<(), CliError> {
    for check in &report.checks {
        writeln!(
            out,
            "{} {} — {}",
            if check.ok { "✓" } else { "✗" },
            check.name,
            check.message
        )
        .map_err(write_error)?;
    }
    Ok(())
}

fn write_event(
    out: &mut dyn Write,
    event: &DaemonEvent,
    json_output: bool,
) -> Result<(), CliError> {
    if json_output {
        print_json(out, event)
    } else {
        writeln!(
            out,
            "{} {} {} {}",
            event.ts_ms,
            event.kind,
            event.project_id.clone().unwrap_or_default(),
            event.payload
        )
        .map_err(write_error)
    }
}

fn print_value(
    out: &mut dyn Write,
    json_output: bool,
    value: &Value,
    human: &str,
) -> Result<(), CliError> {
    if json_output {
        print_json(out, value)
    } else {
        writeln!(out, "{human}").map_err(write_error)
    }
}

fn print_json<T: Serialize + ?Sized>(out: &mut dyn Write, value: &T) -> Result<(), CliError> {
    serde_json::to_writer_pretty(&mut *out, value)
        .map_err(|error| CliError::runtime(format!("failed to encode JSON: {error}")))?;
    writeln!(out).map_err(write_error)
}

fn write_error(error: io::Error) -> CliError {
    CliError::runtime(format!("failed to write output: {error}"))
}

fn drain_complete_sse_frames(buffer: &mut Vec<u8>) -> Result<Vec<String>, CliError> {
    let mut frames = Vec::new();
    loop {
        let Some(index) = buffer.windows(2).position(|window| window == b"\n\n") else {
            break;
        };
        let frame_bytes: Vec<u8> = buffer.drain(..index + 2).collect();
        let frame = std::str::from_utf8(&frame_bytes[..frame_bytes.len() - 2])
            .map_err(|error| CliError::runtime(format!("invalid UTF-8 in event stream: {error}")))?
            .to_string();
        frames.push(frame);
    }
    Ok(frames)
}

fn parse_sse_events(text: &str) -> Vec<DaemonEvent> {
    text.split("\n\n")
        .filter_map(|block| {
            let data = block
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                None
            } else {
                serde_json::from_str(&data).ok()
            }
        })
        .collect()
}

pub fn doctor_exit_code(checks: &[DoctorCheck]) -> i32 {
    if checks.iter().all(|check| check.ok) {
        0
    } else {
        1
    }
}

pub fn usage_text() -> &'static str {
    "refact <SUBCOMMAND> [OPTIONS]\n\nSUBCOMMANDS:\n    ps [--json]\n    projects [--json] [open <path>|pin <id|path>|unpin <id|path>|forget <id|path>]\n    cron [--json] <list|add|run|rm|pause|resume> [--project <id|path>]\n    restart [--json] (--daemon|<id|path>)\n    stop [--json] (--daemon|<id|path>)\n    logs [--json] [-f] [--daemon|<id|path>]\n    events [--json] [-f] [--kind <kind>]\n    status [--json]\n    doctor [--json]\n    version [--json]"
}

pub fn subcommand_usage_text(subcommand: &str) -> Option<&'static str> {
    match subcommand {
        "ps" => Some("refact ps [--json]\n\nList daemon workers."),
        "projects" => Some("refact projects [--json] [open <path>|pin <id|path>|unpin <id|path>|forget <id|path>]\n\nManage daemon project registry."),
        "cron" => Some("refact cron [--json] <list|add|run|rm|pause|resume> [--project <id|path>]\n\nManage worker scheduler jobs through the daemon proxy."),
        "restart" => Some("refact restart [--json] (--daemon|<id|path>)\n\nRestart a project worker or the daemon."),
        "stop" => Some("refact stop [--json] (--daemon|<id|path>)\n\nStop a project worker or the daemon."),
        "logs" => Some("refact logs [--json] [-f] [--daemon|<id|path>]\n\nPrint daemon or worker logs.\n\nOPTIONS:\n    --daemon                    Print daemon logs\n    -f, --follow                Follow log output\n    --json                      Emit the current log as JSON; incompatible with follow"),
        "events" => Some("refact events [--json] [-f] [--kind <kind>]\n\nPrint daemon events.\n\nOPTIONS:\n    --kind <kind>               Filter by event kind\n    -f, --follow                Follow event output\n    --json                      Emit events as JSON; incompatible with follow"),
        "status" => Some("refact status [--json]\n\nPrint daemon health."),
        "doctor" => Some("refact doctor [--json]\n\nDiagnose daemon setup."),
        "version" => Some("refact version [--json]\n\nPrint version and build information."),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::supervisor::WorkerState;

    fn worker(project_id: &str, slug: &str) -> WorkerRow {
        WorkerRow {
            project_id: project_id.to_string(),
            slug: slug.to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: true,
            last_active_ms: 0,
            state: WorkerState::Ready,
            pid: Some(42),
            http_port: Some(8001),
            lsp_port: Some(9001),
            lsp_clients: 2,
            busy_chats: 1,
            exec_running: 0,
            live_proxy_streams: 0,
            cron_next_fire_ms: Some(123),
            idle_deadline_ms: Some(456),
            last_status_report_ms: Some(99),
            last_error: None,
            log_path: "/tmp/demo/logs/worker-demo.log".to_string(),
        }
    }

    #[test]
    fn table_formatting_from_fixture_worker_rows() {
        let table = format_worker_table(&[worker("abcdef123456", "demo")]);
        assert!(table.contains("PROJECT"));
        assert!(table.contains("demo+abcdef12"));
        assert!(table.contains("ready"));
        assert!(table.contains("8001"));
        assert!(table.contains("yes"));
    }

    #[test]
    fn table_formatting_uses_unicode_display_width() {
        let table = format_table(&[
            vec!["NAME".to_string(), "VALUE".to_string()],
            vec!["界".to_string(), "wide".to_string()],
            vec!["aa".to_string(), "ascii".to_string()],
        ]);
        assert_eq!(table, "NAME  VALUE\n界    wide\naa    ascii\n");
    }

    #[test]
    fn parse_logs_rejects_silent_target_and_follow_cases() {
        let options = parse_cli_args(&["logs".into(), "project-id".into()]).unwrap();
        match options.command {
            CliCommand::Logs { target, daemon, .. } => {
                assert_eq!(target.as_deref(), Some("project-id"));
                assert!(!daemon);
            }
            _ => panic!("expected logs command"),
        }

        let error =
            parse_cli_args(&["logs".into(), "--daemon".into(), "project-id".into()]).unwrap_err();
        assert!(error.contains("does not accept a project target"));

        let error = parse_cli_args(&["logs".into(), "--json".into(), "-f".into()]).unwrap_err();
        assert!(error.contains("incompatible"));

        let error = parse_cli_args(&["events".into(), "--json".into(), "-f".into()]).unwrap_err();
        assert!(error.contains("incompatible"));
    }

    #[test]
    fn parse_cron_subcommands_and_json_flag() {
        let options = parse_cli_args(&["cron".into(), "--json".into(), "list".into()]).unwrap();
        assert!(matches!(
            options.command,
            CliCommand::Cron {
                command: CronCommand::List { project: None },
                json: true
            }
        ));

        let options = parse_cli_args(&[
            "cron".into(),
            "pause".into(),
            "--project".into(),
            "abc".into(),
            "job-1".into(),
        ])
        .unwrap();
        assert!(matches!(
            options.command,
            CliCommand::Cron {
                command: CronCommand::Pause {
                    project: Some(_),
                    id
                },
                json: false
            } if id == "job-1"
        ));
    }

    #[test]
    fn parse_cron_add_mirrors_create_request_inputs() {
        let options = parse_cli_args(&[
            "cron".into(),
            "add".into(),
            "--project".into(),
            "abc".into(),
            "--every".into(),
            "5m".into(),
            "--command-argv".into(),
            "[\"echo\",\"hi\"]".into(),
            "--delivery".into(),
            "webhook".into(),
            "--webhook-url".into(),
            "https://example.test/hook".into(),
            "--recurring".into(),
            "true".into(),
            "--durable".into(),
            "Say hi".into(),
        ])
        .unwrap();
        let CliCommand::Cron {
            command: CronCommand::Add(add),
            json: false,
        } = options.command
        else {
            panic!("expected cron add");
        };
        assert_eq!(add.project.as_deref(), Some("abc"));
        assert_eq!(add.every.as_deref(), Some("5m"));
        assert_eq!(
            add.command_argv,
            Some(vec!["echo".to_string(), "hi".to_string()])
        );
        assert!(matches!(add.delivery, CronDeliveryArg::Webhook { .. }));
        assert_eq!(add.recurring, Some(true));
        assert!(add.durable);
        assert_eq!(add.description, "Say hi");
    }

    #[test]
    fn cron_add_request_omits_durable_unless_flagged() {
        let add = CronAddOptions {
            project: None,
            cron: Some("0 * * * *".to_string()),
            every: None,
            at: None,
            tz: None,
            hook_id: None,
            prompt: Some("hello".to_string()),
            command: None,
            command_argv: None,
            cwd: None,
            timeout_secs: None,
            delivery: CronDeliveryArg::Chat,
            recurring: None,
            durable: false,
            isolated: false,
            description: "hello".to_string(),
            chat_id: Some("chat".to_string()),
            mode: None,
        };
        let value = cron_add_request(add.clone());
        assert!(value.get("durable").is_none());
        let mut durable = add;
        durable.durable = true;
        assert_eq!(cron_add_request(durable)["durable"], json!(true));
    }

    #[test]
    fn id_prefix_resolution_picks_unique_project() {
        let projects = vec![ProjectEntry {
            id: "abcdef123456".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: false,
            last_active_ms: 0,
            settings: Default::default(),
        }];
        assert_eq!(resolve_target(&projects, "abc").unwrap(), "abcdef123456");
    }

    #[test]
    fn path_resolution_uses_registered_root() {
        let dir = tempfile::tempdir().unwrap();
        let root =
            crate::files_correction::canonical_path(dir.path().to_string_lossy().to_string());
        let id = "persisted-project-id".to_string();
        let projects = vec![ProjectEntry {
            id: id.clone(),
            slug: "demo".to_string(),
            root,
            pinned: false,
            last_active_ms: 0,
            settings: Default::default(),
        }];
        assert_eq!(
            resolve_target(&projects, &dir.path().to_string_lossy()).unwrap(),
            id
        );
    }

    #[test]
    fn doctor_check_aggregation_exit_code() {
        assert_eq!(doctor_exit_code(&[check("ok", true, "yes")]), 0);
        assert_eq!(doctor_exit_code(&[check("bad", false, "no")]), 1);
    }

    #[tokio::test]
    async fn workers_responsive_ignores_stopped_workers() {
        let mut row = worker("abcdef123456", "demo");
        row.state = WorkerState::Stopped;
        row.http_port = Some(unused_loopback_port().await);

        assert!(workers_responsive(&[row]).await);
    }

    #[test]
    fn parses_events_sse_snapshot() {
        let text =
            "data: {\"ts_ms\":1,\"kind\":\"worker_ready\",\"project_id\":\"p\",\"payload\":{}}\n\n";
        let events = parse_sse_events(text);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "worker_ready");
    }

    #[test]
    fn sse_frame_drain_preserves_split_multibyte() {
        let text = "data: {\"ts_ms\":1,\"kind\":\"worker_ready\",\"project_id\":\"p\",\"payload\":{\"name\":\"项目💡\"}}\n\n";
        let bytes = text.as_bytes();
        let glyph_index = bytes
            .windows("💡".len())
            .position(|window| window == "💡".as_bytes())
            .unwrap();
        let split_index = glyph_index + 2;
        let mut buffer = Vec::new();

        buffer.extend_from_slice(&bytes[..split_index]);
        assert!(drain_complete_sse_frames(&mut buffer).unwrap().is_empty());
        assert_eq!(buffer, bytes[..split_index]);

        buffer.extend_from_slice(&bytes[split_index..]);
        let frames = drain_complete_sse_frames(&mut buffer).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], text.trim_end_matches("\n\n"));
        assert!(!frames[0].contains('�'));

        let events = parse_sse_events(&(frames[0].clone() + "\n\n"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["name"], json!("项目💡"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn sse_frame_drain_errors_on_invalid_utf8() {
        let mut buffer = b"data: ".to_vec();
        buffer.push(0xff);
        buffer.extend_from_slice(b"\n\n");

        let error = drain_complete_sse_frames(&mut buffer).unwrap_err();
        assert!(error.message.contains("invalid UTF-8 in event stream"));
    }

    #[test]
    fn sse_frame_drain_keeps_incomplete_trailing_data() {
        let mut buffer = b"data: {\"ts_ms\":1,\"kind\":\"one\",\"project_id\":null,\"payload\":{}}\n\ndata: partial".to_vec();

        let frames = drain_complete_sse_frames(&mut buffer).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0],
            "data: {\"ts_ms\":1,\"kind\":\"one\",\"project_id\":null,\"payload\":{}}"
        );
        assert_eq!(buffer, b"data: partial");
    }

    #[test]
    fn sse_frame_drain_extracts_multiple_frames() {
        let mut buffer = b"data: {\"ts_ms\":1,\"kind\":\"one\",\"project_id\":null,\"payload\":{}}\n\ndata: {\"ts_ms\":2,\"kind\":\"two\",\"project_id\":null,\"payload\":{}}\n\n".to_vec();

        let frames = drain_complete_sse_frames(&mut buffer).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(buffer.is_empty());

        let events = parse_sse_events(&(frames.join("\n\n") + "\n\n"));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "one");
        assert_eq!(events[1].kind, "two");
    }

    #[tokio::test]
    async fn events_follow_half_dead_listener_fails_bounded() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accept_task = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });

        let mut info = daemon_info(port);
        info.auth_token = None;
        let mut out = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            follow_events(&info, None, false, &mut out),
        )
        .await
        .unwrap();
        let error = result.unwrap_err();
        assert!(error
            .message
            .contains("timed out waiting for event stream headers"));
        accept_task.abort();
    }

    #[tokio::test]
    async fn log_delta_handles_rotation_and_split_multibyte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.log");
        tokio::fs::write(&path, "initial longer log\n")
            .await
            .unwrap();
        let mut state = initial_log_follow_state(&path).await;

        let rotated = format!("{}new-after-rotation\n", "x".repeat(state.offset as usize));
        std::fs::remove_file(&path).unwrap();
        tokio::fs::write(&path, &rotated).await.unwrap();
        assert_eq!(read_log_delta(&path, &mut state).await.unwrap(), rotated);
        assert_eq!(state.offset, rotated.len() as u64);

        tokio::fs::write(&path, b"new\n").await.unwrap();
        assert_eq!(read_log_delta(&path, &mut state).await.unwrap(), "new\n");
        assert_eq!(state.offset, 4);

        let glyph = "💿".as_bytes();
        let mut partial = b"new\n".to_vec();
        partial.extend_from_slice(&glyph[..2]);
        tokio::fs::write(&path, &partial).await.unwrap();
        assert_eq!(read_log_delta(&path, &mut state).await.unwrap(), "");
        assert_eq!(state.offset, 4);

        partial.extend_from_slice(&glyph[2..]);
        partial.push(b'\n');
        tokio::fs::write(&path, &partial).await.unwrap();
        assert_eq!(read_log_delta(&path, &mut state).await.unwrap(), "💿\n");
    }

    fn daemon_info(port: u16) -> DaemonInfo {
        DaemonInfo {
            pid: 1,
            port,
            bind: "127.0.0.1".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            executable_sha256: None,
            auth_token: Some("secret".to_string()),
            started_at_ms: 0,
            hostname_local: "test.local".to_string(),
            urls: crate::daemon::state::DaemonUrls {
                loopback: format!("http://127.0.0.1:{port}"),
                mdns: String::new(),
            },
        }
    }

    async fn unused_loopback_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    }

    struct EnvGuard {
        cache: Option<String>,
        config: Option<String>,
        worker_cmd: Option<String>,
        backoff: Option<String>,
        crash: Option<String>,
    }

    impl EnvGuard {
        fn set_basic(cache: &Path, config: &Path) -> Self {
            let guard = Self {
                cache: std::env::var("REFACT_DAEMON_CACHE_DIR").ok(),
                config: std::env::var("REFACT_DAEMON_CONFIG_DIR").ok(),
                worker_cmd: std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
                backoff: std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
                crash: std::env::var("FAKE_WORKER_CRASH").ok(),
            };
            std::env::set_var("REFACT_DAEMON_CACHE_DIR", cache);
            std::env::set_var("REFACT_DAEMON_CONFIG_DIR", config);
            std::env::remove_var("REFACT_DAEMON_WORKER_CMD");
            std::env::remove_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS");
            std::env::remove_var("FAKE_WORKER_CRASH");
            guard
        }

        fn set(cache: &Path, config: &Path) -> Option<Self> {
            if std::process::Command::new("python3")
                .arg("--version")
                .output()
                .is_err()
            {
                return None;
            }
            let guard = Self {
                cache: std::env::var("REFACT_DAEMON_CACHE_DIR").ok(),
                config: std::env::var("REFACT_DAEMON_CONFIG_DIR").ok(),
                worker_cmd: std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
                backoff: std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
                crash: std::env::var("FAKE_WORKER_CRASH").ok(),
            };
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fake_worker.py");
            std::env::set_var("REFACT_DAEMON_CACHE_DIR", cache);
            std::env::set_var("REFACT_DAEMON_CONFIG_DIR", config);
            std::env::set_var(
                "REFACT_DAEMON_WORKER_CMD",
                shell_words::join(["python3", script.to_string_lossy().as_ref()]),
            );
            std::env::set_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1");
            std::env::remove_var("FAKE_WORKER_CRASH");
            Some(guard)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            restore("REFACT_DAEMON_CACHE_DIR", self.cache.take());
            restore("REFACT_DAEMON_CONFIG_DIR", self.config.take());
            restore("REFACT_DAEMON_WORKER_CMD", self.worker_cmd.take());
            restore("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", self.backoff.take());
            restore("FAKE_WORKER_CRASH", self.crash.take());
        }
    }

    fn restore(key: &str, value: Option<String>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    async fn wait_for_daemon_info(path: &Path) -> crate::daemon::state::DaemonInfo {
        for _ in 0..100 {
            if let Ok(Some(info)) = crate::daemon::state::read_daemon_info(path).await {
                return info;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("daemon did not start")
    }

    async fn start_test_daemon(
        cache_dir: &tempfile::TempDir,
    ) -> (
        crate::daemon::state::DaemonInfo,
        tokio::task::JoinHandle<i32>,
    ) {
        let paths = crate::daemon::RuntimePaths::in_dir(&crate::daemon::paths::daemon_dir());
        let config = crate::daemon::config::DaemonConfig {
            bind: "127.0.0.1".to_string(),
            port: 0,
            ..crate::daemon::config::DaemonConfig::default()
        };
        let task = tokio::spawn(async move {
            crate::daemon::run_daemon_entry_with_paths(config, paths, false, false).await
        });
        let daemon_json = cache_dir.path().join("daemon").join("daemon.json");
        (wait_for_daemon_info(&daemon_json).await, task)
    }

    async fn shutdown_test_daemon(
        info: &crate::daemon::state::DaemonInfo,
        task: tokio::task::JoinHandle<i32>,
    ) {
        client::shutdown_daemon(info, "test").await.unwrap();
        assert_eq!(task.await.unwrap(), 0);
    }

    async fn retry_open_project(info: &crate::daemon::state::DaemonInfo, root: &Path) -> Value {
        let mut last_error = None;
        for _ in 0..20 {
            match client::post_json::<_, Value>(
                info,
                "/daemon/v1/projects/open",
                &json!({"root": root}),
            )
            .await
            {
                Ok(value) => return value,
                Err(error) => {
                    last_error = Some(error);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
        panic!("daemon project open failed: {:?}", last_error);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn stop_daemon_missing_file_does_not_spawn() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let Some(_guard) = EnvGuard::set(cache_dir.path(), config_dir.path()) else {
            return;
        };
        let mut out = Vec::new();
        assert_eq!(stop_daemon(false, &mut out).await.unwrap(), 0);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "no daemon running (missing)\n"
        );
        assert!(!crate::daemon::paths::daemon_json_path().exists());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn stop_daemon_stale_file_does_not_spawn() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let Some(_guard) = EnvGuard::set(cache_dir.path(), config_dir.path()) else {
            return;
        };
        crate::daemon::state::write_daemon_info_atomic(
            &crate::daemon::paths::daemon_json_path(),
            &daemon_info(9),
        )
        .await
        .unwrap();
        let mut out = Vec::new();
        assert_eq!(stop_daemon(true, &mut out).await.unwrap(), 0);
        assert_eq!(
            serde_json::from_slice::<Value>(&out).unwrap(),
            json!({"stopped": false, "reason": "stale"})
        );
        let info =
            crate::daemon::state::read_daemon_info(&crate::daemon::paths::daemon_json_path())
                .await
                .unwrap();
        assert!(info.is_some());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn project_stop_and_restart_missing_daemon_do_not_spawn() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_basic(cache_dir.path(), config_dir.path());

        for command in [
            CliCommand::Stop {
                target: Some("project".to_string()),
                daemon: false,
                json: true,
            },
            CliCommand::Restart {
                target: Some("project".to_string()),
                daemon: false,
                json: true,
            },
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = run_with_io(CliOptions { command }, &mut stdout, &mut stderr).await;

            assert_eq!(code, 1);
            assert!(stderr.is_empty());
            let value = serde_json::from_slice::<Value>(&stdout).unwrap();
            assert_eq!(value["ok"], false);
            assert!(value["error"]
                .as_str()
                .unwrap()
                .contains("daemon not running"));
            assert!(!crate::daemon::paths::daemon_json_path().exists());
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn run_with_io_json_error_uses_stdout_and_silent_stderr() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_basic(cache_dir.path(), config_dir.path());
        tokio::fs::create_dir_all(crate::daemon::paths::daemon_dir())
            .await
            .unwrap();
        tokio::fs::write(crate::daemon::paths::daemon_json_path(), b"not json")
            .await
            .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_io(
            CliOptions {
                command: CliCommand::Status { json: true },
            },
            &mut stdout,
            &mut stderr,
        )
        .await;
        assert_eq!(code, 1);
        assert!(stderr.is_empty());
        let value = serde_json::from_slice::<Value>(&stdout).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["exit_code"], 1);
        assert!(value["error"]
            .as_str()
            .unwrap()
            .contains("failed to read daemon.json"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn status_json_missing_daemon_json_is_passive_unreachable() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_basic(cache_dir.path(), config_dir.path());

        let mut out = Vec::new();
        assert_eq!(run_status(true, &mut out).await.unwrap(), 1);
        assert_eq!(
            serde_json::from_slice::<Value>(&out).unwrap(),
            json!({"reachable": false, "reason": "daemon.json not found"})
        );
        assert!(!crate::daemon::paths::daemon_json_path().exists());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn status_human_missing_daemon_json_is_passive_unreachable() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_basic(cache_dir.path(), config_dir.path());

        let mut out = Vec::new();
        assert_eq!(run_status(false, &mut out).await.unwrap(), 1);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "daemon not running: daemon.json not found\n"
        );
        assert!(!crate::daemon::paths::daemon_json_path().exists());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn version_json_unreachable_daemon_is_valid_json() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_basic(cache_dir.path(), config_dir.path());
        let port = unused_loopback_port().await;
        crate::daemon::state::write_daemon_info_atomic(
            &crate::daemon::paths::daemon_json_path(),
            &daemon_info(port),
        )
        .await
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_io(
            CliOptions {
                command: CliCommand::Version { json: true },
            },
            &mut stdout,
            &mut stderr,
        )
        .await;
        assert_eq!(code, 0);
        assert!(stderr.is_empty());
        let value = serde_json::from_slice::<Value>(&stdout).unwrap();
        assert_eq!(value["client"], env!("CARGO_PKG_VERSION"));
        assert!(value["daemon"].is_null());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn stop_daemon_auth_enabled_live_daemon_shuts_down() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let Some(_guard) = EnvGuard::set(cache_dir.path(), config_dir.path()) else {
            return;
        };
        let paths = crate::daemon::RuntimePaths::in_dir(&crate::daemon::paths::daemon_dir());
        let config = crate::daemon::config::DaemonConfig {
            bind: "127.0.0.1".to_string(),
            port: 0,
            auth: crate::daemon::config::AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..crate::daemon::config::DaemonConfig::default()
        };
        let task = tokio::spawn(async move {
            crate::daemon::run_daemon_entry_with_paths(config, paths, false, false).await
        });
        let daemon_json = cache_dir.path().join("daemon").join("daemon.json");
        let info = wait_for_daemon_info(&daemon_json).await;
        assert_eq!(info.auth_token.as_deref(), Some("secret"));

        let mut out = Vec::new();
        assert_eq!(stop_daemon(false, &mut out).await.unwrap(), 0);
        assert_eq!(String::from_utf8(out).unwrap(), "daemon stopped\n");
        assert_eq!(task.await.unwrap(), 0);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn ps_projects_logs_events_status_roundtrip_live_daemon() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let Some(_guard) = EnvGuard::set(cache_dir.path(), config_dir.path()) else {
            return;
        };
        let project_dir = tempfile::tempdir().unwrap();
        let (info, task) = start_test_daemon(&cache_dir).await;

        let mut out = Vec::new();
        assert_eq!(
            run_inner(
                CliOptions {
                    command: CliCommand::Projects {
                        command: ProjectsCommand::Open {
                            path: project_dir.path().to_path_buf()
                        },
                        json: true
                    }
                },
                &mut out
            )
            .await
            .unwrap(),
            0
        );
        let opened: Value = serde_json::from_slice(&out).unwrap();
        let project_id = opened["project_id"].as_str().unwrap().to_string();
        let slug = opened["slug"].as_str().unwrap().to_string();

        out.clear();
        assert_eq!(
            run_inner(
                CliOptions {
                    command: CliCommand::Projects {
                        command: ProjectsCommand::List,
                        json: true
                    }
                },
                &mut out
            )
            .await
            .unwrap(),
            0
        );
        let projects = serde_json::from_slice::<Value>(&out).unwrap();
        assert!(projects["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|project| { project["id"].as_str() == Some(project_id.as_str()) }));
        assert!(projects.as_array().is_none());

        out.clear();
        let daemon_marker = "daemon-log-marker-B2\n";
        let worker_marker = "worker-log-marker-B2\n";
        tokio::fs::create_dir_all(crate::daemon::paths::logs_dir())
            .await
            .unwrap();
        tokio::fs::write(crate::daemon::paths::daemon_log_path(), daemon_marker)
            .await
            .unwrap();
        tokio::fs::write(
            crate::daemon::paths::logs_dir().join(format!("worker-{slug}.log")),
            worker_marker,
        )
        .await
        .unwrap();
        assert_eq!(
            run_inner(
                CliOptions {
                    command: CliCommand::Ps { json: false }
                },
                &mut out
            )
            .await
            .unwrap(),
            0
        );
        let ps = String::from_utf8(out.clone()).unwrap();
        assert!(ps.contains("daemon pid="));
        assert!(ps.contains("PROJECT"));

        out.clear();
        assert_eq!(
            run_inner(
                CliOptions {
                    command: CliCommand::Projects {
                        command: ProjectsCommand::Pin {
                            target: project_id[..6].to_string()
                        },
                        json: true
                    }
                },
                &mut out
            )
            .await
            .unwrap(),
            0
        );
        assert!(serde_json::from_slice::<Value>(&out).unwrap()["pinned"]
            .as_bool()
            .unwrap());

        out.clear();
        assert_eq!(
            run_inner(
                CliOptions {
                    command: CliCommand::Logs {
                        target: Some(project_id.clone()),
                        daemon: false,
                        follow: false,
                        json: true
                    }
                },
                &mut out
            )
            .await
            .unwrap(),
            0
        );
        let log = serde_json::from_slice::<Value>(&out).unwrap()["log"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(log.contains(worker_marker));
        assert!(!log.contains(daemon_marker));

        out.clear();
        assert_eq!(
            run_inner(
                CliOptions {
                    command: CliCommand::Events {
                        kind: Some("project_opened".to_string()),
                        follow: false,
                        json: true
                    }
                },
                &mut out
            )
            .await
            .unwrap(),
            0
        );
        let events = serde_json::from_slice::<Vec<DaemonEvent>>(&out).unwrap();
        assert!(events.iter().any(|event| event.kind == "project_opened"));

        out.clear();
        assert_eq!(
            run_inner(
                CliOptions {
                    command: CliCommand::Status { json: false }
                },
                &mut out
            )
            .await
            .unwrap(),
            0
        );
        assert!(String::from_utf8(out).unwrap().contains("daemon healthy"));

        shutdown_test_daemon(&info, task).await;
    }

    #[tokio::test]
    #[serial_test::serial]
    #[cfg_attr(
        windows,
        ignore = "Windows artifact runners can leave proxy worker unavailable"
    )]
    async fn cron_cli_dispatches_through_daemon_proxy() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let Some(_guard) = EnvGuard::set(cache_dir.path(), config_dir.path()) else {
            return;
        };
        let project_dir = tempfile::tempdir().unwrap();
        let (info, task) = start_test_daemon(&cache_dir).await;
        let opened: Value = retry_open_project(&info, project_dir.path()).await;
        let project_id = opened["project_id"].as_str().unwrap().to_string();

        let mut out = Vec::new();
        run_inner(
            CliOptions {
                command: CliCommand::Cron {
                    command: CronCommand::List {
                        project: Some(project_id.clone()),
                    },
                    json: true,
                },
            },
            &mut out,
        )
        .await
        .unwrap();
        let listed = serde_json::from_slice::<Value>(&out).unwrap();
        assert_eq!(listed[0]["id"], "job-1");

        out.clear();
        run_inner(
            CliOptions {
                command: CliCommand::Cron {
                    command: CronCommand::Pause {
                        project: Some(project_id.clone()),
                        id: "job-1".to_string(),
                    },
                    json: true,
                },
            },
            &mut out,
        )
        .await
        .unwrap();
        let paused = serde_json::from_slice::<Value>(&out).unwrap();
        assert_eq!(paused["method"], "PATCH");
        assert_eq!(paused["path"], "/v1/scheduler/cron/job-1");
        assert!(paused["body_text"]
            .as_str()
            .unwrap()
            .contains("\"enabled\":false"));

        shutdown_test_daemon(&info, task).await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn doctor_catches_dead_daemon_version_mismatch() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let Some(_guard) = EnvGuard::set(cache_dir.path(), config_dir.path()) else {
            return;
        };
        let (info, task) = start_test_daemon(&cache_dir).await;
        let project_dir = tempfile::tempdir().unwrap();
        let opened: Value = retry_open_project(&info, project_dir.path()).await;
        let project_id = opened["project_id"].as_str().unwrap().to_string();
        let _: Value = client::delete_json(&info, &format!("/daemon/v1/projects/{project_id}"))
            .await
            .unwrap();
        let report = doctor_report().await;
        assert_eq!(report.exit_code(), 0);
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "binary path" && check.ok));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "worker count" && check.ok));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "project roots" && check.ok));
        shutdown_test_daemon(&info, task).await;

        let mut stale = info.clone();
        stale.version = "0.0.0".to_string();
        crate::daemon::state::write_daemon_info_atomic(
            &crate::daemon::paths::daemon_json_path(),
            &stale,
        )
        .await
        .unwrap();
        let report = doctor_report().await;
        assert_eq!(report.exit_code(), 1);
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "daemon reachable" && !check.ok));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "version match" && !check.ok));
    }
}
