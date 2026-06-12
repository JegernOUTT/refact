use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::client::{self, DaemonClientError};
use crate::daemon::events::DaemonEvent;
use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::{DaemonInfo, WorkerRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Ps {
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
pub struct CliOptions {
    pub command: CliCommand,
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
    let follow = take_flag(args, "-f")? || take_flag(args, "--follow")?;
    let daemon = take_flag(args, "--daemon")?;
    let target = if args.is_empty() {
        None
    } else {
        Some(args.remove(0))
    };
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
    let follow = take_flag(args, "-f")? || take_flag(args, "--follow")?;
    let kind = take_option(args, "--kind")?;
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
    match run_inner(options, &mut io::stdout()).await {
        Ok(code) => code,
        Err(error) => {
            let _ = writeln!(io::stderr(), "{}", error.message);
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
        CliCommand::Restart { target, json, .. } => {
            let daemon = ensure_daemon().await?;
            let projects = list_projects(&daemon).await?;
            let id = resolve_target(&projects, target.as_deref().unwrap_or_default())?;
            let worker: Value =
                client_post_empty(&daemon, &format!("/daemon/v1/projects/{id}/restart")).await?;
            print_value(out, json, &worker, "restarted")?;
            Ok(0)
        }
        CliCommand::Stop { target, json, .. } => {
            let daemon = ensure_daemon().await?;
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
                print_json(out, &projects)?;
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
    let path = if daemon_logs || target.is_none() {
        "/daemon/v1/logs?tail=200".to_string()
    } else {
        let projects = list_projects(&daemon).await?;
        let id = resolve_target(&projects, target.as_deref().unwrap_or_default())?;
        format!("/daemon/v1/logs?project_id={id}&tail=200")
    };
    let text = client::get_text(&daemon, &path)
        .await
        .map_err(client_error)?;
    if json_output {
        print_json(out, &json!({"log": text}))?;
    } else {
        write!(out, "{text}").map_err(write_error)?;
        if follow {
            follow_logs(&daemon, &path, text.len(), out).await?;
        }
    }
    Ok(0)
}

async fn follow_logs(
    daemon: &DaemonInfo,
    path: &str,
    mut seen: usize,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                let text = client::get_text(daemon, path).await.map_err(client_error)?;
                if text.len() > seen {
                    write!(out, "{}", &text[seen..]).map_err(write_error)?;
                    seen = text.len();
                }
            }
        }
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
    let client = reqwest::Client::new();
    let request = match &daemon.auth_token {
        Some(token) => client.get(url).bearer_auth(token),
        None => client.get(url),
    };
    let response = request.send().await.map_err(|error| {
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
    let mut buffer = String::new();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            next = stream.next() => {
                let Some(chunk) = next else { return Ok(()); };
                let chunk = chunk.map_err(|error| CliError::runtime(format!("daemon event stream failed: {error}")))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(index) = buffer.find("\n\n") {
                    let block = buffer[..index].to_string();
                    buffer = buffer[index + 2..].to_string();
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

async fn run_status(json_output: bool, out: &mut dyn Write) -> Result<i32, CliError> {
    let info = ensure_daemon().await?;
    let status: DaemonStatus = client_get(&info, "/daemon/v1/status").await?;
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

async fn run_version(json_output: bool, out: &mut dyn Write) -> Result<i32, CliError> {
    let daemon = match client::read_daemon_json().await {
        Some(info) if client::ping_daemon(&info).await => Some(info.version),
        _ => None,
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
    if let Some(info) = client::read_daemon_json().await {
        if client::ping_daemon(&info).await {
            client::shutdown_daemon(&info, "restart")
                .await
                .map_err(CliError::runtime)?;
            client::wait_for_daemon_stop(&info, Duration::from_secs(15))
                .await
                .map_err(CliError::runtime)?;
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
    let info = ensure_daemon().await?;
    client::shutdown_daemon(&info, "stop")
        .await
        .map_err(CliError::runtime)?;
    if json_output {
        print_json(out, &json!({"stopped": true}))?;
    } else {
        writeln!(out, "daemon stopped").map_err(write_error)?;
    }
    Ok(0)
}

async fn doctor_report() -> DoctorReport {
    let daemon_json_path = crate::daemon::paths::daemon_json_path();
    let mut checks = Vec::new();
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
        let reachable = client::ping_daemon(info).await;
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
                    let responsive = workers_responsive(info, &workers).await;
                    let missing = workers
                        .iter()
                        .filter(|row| !row.root.exists())
                        .map(|row| row.slug.clone())
                        .collect::<Vec<_>>();
                    checks.push(check(
                        "workers responsive",
                        responsive,
                        format!("{} workers", workers.len()),
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
                Err(error) => checks.push(check("workers responsive", false, error.to_string())),
            }
        } else {
            checks.push(check("workers responsive", false, "daemon unreachable"));
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
    }
    checks.push(check(
        "lock file",
        crate::daemon::paths::lock_path().exists(),
        crate::daemon::paths::lock_path().display().to_string(),
    ));
    DoctorReport { checks }
}

async fn workers_responsive(info: &DaemonInfo, workers: &[WorkerRow]) -> bool {
    for worker in workers {
        if worker.http_port.is_some()
            && client::get_text(info, &format!("/p/{}/v1/ping", worker.project_id))
                .await
                .is_err()
        {
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
    let id = project_id_for_path(&root);
    if projects.iter().any(|project| project.id == id) {
        Ok(id)
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

fn project_id_for_path(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().as_bytes());
    hex::encode(&hasher.finalize()[..6])
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
            widths[index] = widths[index].max(cell.len());
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
                out.push_str(&" ".repeat(widths[index].saturating_sub(cell.len())));
            }
        }
        out.push('\n');
    }
    out
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
    "refact <SUBCOMMAND> [OPTIONS]\n\nSUBCOMMANDS:\n    ps [--json]\n    projects [--json] [open <path>|pin <id|path>|unpin <id|path>|forget <id|path>]\n    restart [--json] (--daemon|<id|path>)\n    stop [--json] (--daemon|<id|path>)\n    logs [--json] [-f] [--daemon|<id|path>]\n    events [--json] [-f] [--kind <kind>]\n    status [--json]\n    doctor [--json]\n    version [--json]"
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
    fn path_resolution_uses_canonical_hash() {
        let dir = tempfile::tempdir().unwrap();
        let root =
            crate::files_correction::canonical_path(dir.path().to_string_lossy().to_string());
        let id = project_id_for_path(&root);
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

    #[test]
    fn parses_events_sse_snapshot() {
        let text =
            "data: {\"ts_ms\":1,\"kind\":\"worker_ready\",\"project_id\":\"p\",\"payload\":{}}\n\n";
        let events = parse_sse_events(text);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "worker_ready");
    }

    struct EnvGuard {
        cache: Option<String>,
        config: Option<String>,
        worker_cmd: Option<String>,
        backoff: Option<String>,
        crash: Option<String>,
    }

    impl EnvGuard {
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
                format!("python3 {}", script.display()),
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
        for _ in 0..100 {
            if let Ok(Some(info)) = crate::daemon::state::read_daemon_info(&daemon_json).await {
                return (info, task);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("daemon did not start")
    }

    async fn shutdown_test_daemon(
        info: &crate::daemon::state::DaemonInfo,
        task: tokio::task::JoinHandle<i32>,
    ) {
        client::shutdown_daemon(info, "test").await.unwrap();
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

        out.clear();
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
        assert!(serde_json::from_slice::<Value>(&out).unwrap()["log"].is_string());

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
    async fn doctor_catches_dead_daemon_version_mismatch_and_missing_project_root() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let Some(_guard) = EnvGuard::set(cache_dir.path(), config_dir.path()) else {
            return;
        };
        let project_dir = tempfile::tempdir().unwrap();
        let (info, task) = start_test_daemon(&cache_dir).await;
        let _: Value = client::post_json(
            &info,
            "/daemon/v1/projects/open",
            &json!({"root": project_dir.path()}),
        )
        .await
        .unwrap();
        drop(project_dir);
        let report = doctor_report().await;
        assert_eq!(report.exit_code(), 1);
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "project roots" && !check.ok));
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
