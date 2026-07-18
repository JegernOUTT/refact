use std::ffi::OsString;
use std::io::Write;

use structopt::clap::ErrorKind;
use structopt::StructOpt;

use crate::global_context::CommandLine;

#[derive(Debug, Clone)]
pub enum RefactCliCommand {
    Worker(CommandLine),
    Daemon { foreground: bool },
    Run(crate::daemon::run_cmd::RunOptions),
    Tui(TuiOptions),
    Control(crate::daemon::cli::CliOptions),
    SelfUpdate(crate::self_update::SelfUpdateOptions),
    Help(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiOptions {
    pub project: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliDispatchError {
    pub message: String,
    pub exit_code: i32,
    pub use_stderr: bool,
    pub kind: Option<&'static str>,
    pub json: bool,
}

#[derive(Debug, Clone)]
pub enum DispatchResult {
    Worker(CommandLine),
    Daemon { foreground: bool },
    Run(crate::daemon::run_cmd::RunOptions),
    Tui(TuiOptions),
    Control(crate::daemon::cli::CliOptions),
    SelfUpdate(crate::self_update::SelfUpdateOptions),
    Exit(i32),
}

impl CliDispatchError {
    pub fn exit(self) -> ! {
        if self.json {
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                match self.kind {
                    Some(kind) =>
                        serde_json::json!({"ok": false, "error": self.message, "kind": kind, "exit_code": self.exit_code}),
                    None =>
                        serde_json::json!({"ok": false, "error": self.message, "exit_code": self.exit_code}),
                }
            );
        } else if self.use_stderr {
            let _ = writeln!(std::io::stderr(), "{}", self.message);
        } else {
            let _ = writeln!(std::io::stdout(), "{}", self.message);
        }
        std::process::exit(self.exit_code);
    }
}

pub fn parse_from_env() -> Result<RefactCliCommand, CliDispatchError> {
    parse_from(std::env::args_os())
}

pub fn parse_from<I>(iter: I) -> Result<RefactCliCommand, CliDispatchError>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let args: Vec<OsString> = iter.into_iter().map(Into::into).collect();
    let Some(subcommand) = args.get(1) else {
        return Ok(RefactCliCommand::Tui(TuiOptions { project: None }));
    };
    match subcommand.to_string_lossy().as_ref() {
        "worker" => parse_worker(args),
        "daemon" => parse_daemon(&args),
        "run" => parse_run(&args),
        "tui" => parse_tui(&args),
        "self-update" => parse_self_update(&args),
        "ui" | "ps" | "projects" | "cron" | "restart" | "stop" | "logs" | "events" | "status"
        | "doctor" | "version" => parse_control(&args),
        "--version" | "-V" => parse_control(&[OsString::from("refact"), OsString::from("version")]),
        "help" | "--help" | "-h" => Ok(RefactCliCommand::Help(help_text())),
        other => Err(if contains_json(args.iter().skip(1)) {
            json_usage_error(format!("unknown subcommand `{}`", other))
        } else {
            usage_error(format!("unknown subcommand `{}`", other))
        }),
    }
}

pub fn dispatch(command: RefactCliCommand) -> DispatchResult {
    match command {
        RefactCliCommand::Worker(cmdline) => DispatchResult::Worker(cmdline),
        RefactCliCommand::Daemon { foreground } => DispatchResult::Daemon { foreground },
        RefactCliCommand::Run(options) => DispatchResult::Run(options),
        RefactCliCommand::Tui(options) => DispatchResult::Tui(options),
        RefactCliCommand::Control(options) => DispatchResult::Control(options),
        RefactCliCommand::SelfUpdate(options) => DispatchResult::SelfUpdate(options),
        RefactCliCommand::Help(text) => {
            println!("{}", text);
            DispatchResult::Exit(0)
        }
    }
}

pub fn help_text() -> &'static str {
    "refact <SUBCOMMAND> [OPTIONS]\n\nUSAGE:\n    refact                       Open the full-screen TUI\n    refact <SUBCOMMAND> [OPTIONS]\n\nSUBCOMMANDS:\n    ui [<path>] [--json] [--no-open]\n                                Open the dashboard or a project workspace\n    tui [--project <path>]      Open the full-screen TUI\n    worker [engine flags...]    Run the refact worker engine\n    daemon [--foreground]       Run the refact daemon\n    run [OPTIONS] <prompt>      Run one headless chat turn through the daemon\n    ps                          List daemon workers\n    projects                    Manage daemon project registry\n    cron                        Manage scheduler jobs\n    restart                     Restart a project worker or daemon\n    stop                        Stop a project worker or daemon\n    logs                        Print daemon or worker logs\n    events                      Print daemon events\n    status                      Print daemon health\n    doctor                      Diagnose daemon setup\n    version                     Print version and build information\n    self-update [OPTIONS]       Update this refact binary from GitHub Releases\n\nTUI OPTIONS:\n    --project <path>            Project root (default: cwd)\n\nRUN OPTIONS:\n    --project <path>            Project root (default: cwd)\n    --mode agent|explore        Chat mode (default: agent)\n    --model <model>             Model id\n    --approve deny|ask|auto     Tool approval policy (default: deny)\n    --json                      Emit final JSON instead of streaming text\n    --timeout-secs <N>          Timeout in seconds (default: 600)\n\nAll management commands support --json. Run `refact worker --help` for engine flags."
}

pub fn daemon_help_text() -> &'static str {
    "refact daemon [--foreground]\n\nRun the refact daemon control process.\n\nOPTIONS:\n    --foreground                Run in the foreground instead of detaching\n    -h, --help                  Print this help text"
}

pub fn tui_help_text() -> &'static str {
    "refact tui [--project <path>]\n\nOpen the full-screen TUI.\n\nOPTIONS:\n    --project <path>            Project root (default: cwd)\n    -h, --help                  Print this help text"
}

pub fn run_help_text() -> &'static str {
    "refact run [OPTIONS] <prompt>\n\nRun one headless chat turn through the daemon.\n\nOPTIONS:\n    --project <path>            Project root (default: cwd)\n    --mode agent|explore        Chat mode (default: agent)\n    --model <model>             Model id\n    --approve deny|ask|auto     Tool approval policy (default: deny)\n    --json                      Emit final JSON instead of streaming text\n    --timeout-secs <N>          Timeout in seconds (default: 600)\n    -h, --help                  Print this help text"
}

pub fn self_update_help_text() -> &'static str {
    "refact self-update [OPTIONS]\n\nUpdate this refact binary from GitHub Releases.\n\nOPTIONS:\n    --check                     Print current/latest versions without downloading\n    --version <v>               Install an explicit engine release version\n    --force                     Reinstall latest even when it is not newer\n    --quiet                     Suppress successful text output\n    --json                      Emit JSON output\n    -h, --help                  Print this help text"
}

pub fn version_text() -> String {
    let mut lines = vec![format!("refact {}", env!("CARGO_PKG_VERSION"))];
    lines.extend(
        crate::http::routers::info::get_build_info()
            .into_iter()
            .map(|(key, value)| format!("{:>20} {}", key, value)),
    );
    lines.join("\n")
}

fn parse_worker(args: Vec<OsString>) -> Result<RefactCliCommand, CliDispatchError> {
    let mut worker_args = Vec::with_capacity(args.len().saturating_sub(1));
    worker_args.push(OsString::from("refact worker"));
    worker_args.extend(args.into_iter().skip(2));
    CommandLine::from_iter_safe(worker_args)
        .map(RefactCliCommand::Worker)
        .map_err(clap_error)
}

fn parse_daemon(args: &[OsString]) -> Result<RefactCliCommand, CliDispatchError> {
    let mut foreground = false;
    for arg in args.iter().skip(2) {
        match arg.to_string_lossy().as_ref() {
            "--foreground" => foreground = true,
            "--help" | "-h" => return Ok(RefactCliCommand::Help(daemon_help_text())),
            other => {
                return Err(usage_error(format!(
                    "unexpected daemon argument `{}`",
                    other
                )))
            }
        }
    }
    Ok(RefactCliCommand::Daemon { foreground })
}

fn parse_run(args: &[OsString]) -> Result<RefactCliCommand, CliDispatchError> {
    if contains_help(args.iter().skip(2)) {
        return Ok(RefactCliCommand::Help(run_help_text()));
    }
    crate::daemon::run_cmd::parse_run_args(&args.iter().skip(2).cloned().collect::<Vec<_>>())
        .map(RefactCliCommand::Run)
        .map_err(|message| {
            if contains_json(args.iter().skip(2)) {
                json_usage_error(message)
            } else {
                usage_error(message)
            }
        })
}

fn parse_tui(args: &[OsString]) -> Result<RefactCliCommand, CliDispatchError> {
    let mut project = None;
    let mut i = 2usize;
    while i < args.len() {
        let value = args[i].to_string_lossy();
        match value.as_ref() {
            "--project" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    return Err(usage_error("--project requires a path".to_string()));
                };
                project = Some(std::path::PathBuf::from(path));
            }
            "--help" | "-h" => return Ok(RefactCliCommand::Help(tui_help_text())),
            other => return Err(usage_error(format!("unexpected tui argument `{}`", other))),
        }
        i += 1;
    }
    Ok(RefactCliCommand::Tui(TuiOptions { project }))
}

fn parse_self_update(args: &[OsString]) -> Result<RefactCliCommand, CliDispatchError> {
    if contains_help(args.iter().skip(2)) {
        return Ok(RefactCliCommand::Help(self_update_help_text()));
    }
    crate::self_update::parse_self_update_args(&args.iter().skip(2).cloned().collect::<Vec<_>>())
        .map(RefactCliCommand::SelfUpdate)
        .map_err(usage_error)
}

fn parse_control(args: &[OsString]) -> Result<RefactCliCommand, CliDispatchError> {
    if contains_help(args.iter().skip(2)) {
        let subcommand = args
            .get(1)
            .and_then(|arg| arg.to_str())
            .and_then(crate::daemon::cli::subcommand_usage_text)
            .unwrap_or_else(help_text);
        return Ok(RefactCliCommand::Help(subcommand));
    }
    crate::daemon::cli::parse_cli_args(&args.iter().skip(1).cloned().collect::<Vec<_>>())
        .map(RefactCliCommand::Control)
        .map_err(|message| {
            if contains_json(args.iter().skip(2)) {
                json_usage_error(message)
            } else {
                usage_error(message)
            }
        })
}

fn contains_help<'a>(args: impl Iterator<Item = &'a OsString>) -> bool {
    args.filter_map(|arg| arg.to_str())
        .any(|arg| arg == "--help" || arg == "-h")
}

fn contains_json<'a>(args: impl Iterator<Item = &'a OsString>) -> bool {
    args.filter_map(|arg| arg.to_str())
        .any(|arg| arg == "--json")
}

fn clap_error(error: structopt::clap::Error) -> CliDispatchError {
    let exit_code = match error.kind {
        ErrorKind::HelpDisplayed | ErrorKind::VersionDisplayed => 0,
        _ => 1,
    };
    let use_stderr = error.use_stderr();
    CliDispatchError {
        message: error.message,
        exit_code,
        use_stderr,
        kind: None,
        json: false,
    }
}

fn usage_error(message: String) -> CliDispatchError {
    CliDispatchError {
        message: format!("error: {}\n\n{}", message, help_text()),
        exit_code: 2,
        use_stderr: true,
        kind: None,
        json: false,
    }
}

fn json_usage_error(message: String) -> CliDispatchError {
    CliDispatchError {
        message: format!("error: {}\n\n{}", message, help_text()),
        exit_code: 2,
        use_stderr: false,
        kind: Some("usage"),
        json: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_worker_tail_flags() {
        let command = parse_from([
            "refact",
            "worker",
            "--http-port",
            "1234",
            "-w",
            "/tmp",
            "--daemon-endpoint",
            "http://127.0.0.1:8488",
            "--project-id",
            "abc123",
        ])
        .unwrap();
        match command {
            RefactCliCommand::Worker(cmdline) => {
                assert_eq!(cmdline.http_port, 1234);
                assert_eq!(cmdline.workspace_folder, "/tmp");
                assert_eq!(cmdline.daemon_endpoint, "http://127.0.0.1:8488");
                assert_eq!(cmdline.project_id, "abc123");
            }
            _ => panic!("expected worker command"),
        }
    }

    #[test]
    fn parse_version() {
        match parse_from(["refact", "version"]).unwrap() {
            RefactCliCommand::Control(options) => {
                assert!(matches!(
                    options.command,
                    crate::daemon::cli::CliCommand::Version { .. }
                ));
            }
            _ => panic!("expected version control command"),
        }
    }

    #[test]
    fn parse_cron_control_command() {
        match parse_from(["refact", "cron", "list", "--json"]).unwrap() {
            RefactCliCommand::Control(options) => {
                assert!(matches!(
                    options.command,
                    crate::daemon::cli::CliCommand::Cron { json: true, .. }
                ));
            }
            _ => panic!("expected cron control command"),
        }
    }

    #[test]
    fn parse_ui_control_command() {
        match parse_from(["refact", "ui", "/tmp/project", "--no-open"]).unwrap() {
            RefactCliCommand::Control(options) => {
                assert!(matches!(
                    options.command,
                    crate::daemon::cli::CliCommand::Ui {
                        path: Some(path),
                        json: false,
                        no_open: true,
                    } if path == std::path::PathBuf::from("/tmp/project")
                ));
            }
            _ => panic!("expected ui control command"),
        }
    }
    #[test]
    fn parse_daemon_foreground() {
        assert!(matches!(
            parse_from(["refact", "daemon", "--foreground"]).unwrap(),
            RefactCliCommand::Daemon { foreground: true }
        ));
    }

    #[test]
    fn parse_self_update_options() {
        let command = parse_from([
            "refact",
            "self-update",
            "--check",
            "--version",
            "v8.2.0",
            "--force",
            "--quiet",
            "--json",
        ])
        .unwrap();
        match command {
            RefactCliCommand::SelfUpdate(options) => {
                assert!(options.check);
                assert_eq!(options.version.as_deref(), Some("8.2.0"));
                assert!(options.force);
                assert!(options.quiet);
                assert!(options.json);
            }
            _ => panic!("expected self-update command"),
        }
    }

    #[test]
    fn parse_run_defaults() {
        let command = parse_from(["refact", "run", "say hi"]).unwrap();
        match command {
            RefactCliCommand::Run(options) => {
                assert_eq!(options.prompt, "say hi");
                assert_eq!(
                    options.approve,
                    crate::daemon::run_cmd::ApprovalPolicy::Deny
                );
                assert_eq!(options.mode, crate::daemon::run_cmd::RunMode::Agent);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parse_run_full_options() {
        let command = parse_from([
            "refact",
            "run",
            "--project",
            "/tmp/project",
            "--mode",
            "explore",
            "--model",
            "m",
            "--approve",
            "auto",
            "--json",
            "--timeout-secs",
            "9",
            "do work",
        ])
        .unwrap();
        match command {
            RefactCliCommand::Run(options) => {
                assert_eq!(
                    options.project,
                    Some(std::path::PathBuf::from("/tmp/project"))
                );
                assert_eq!(options.mode, crate::daemon::run_cmd::RunMode::Explore);
                assert_eq!(options.model.as_deref(), Some("m"));
                assert_eq!(
                    options.approve,
                    crate::daemon::run_cmd::ApprovalPolicy::Auto
                );
                assert!(options.json);
                assert_eq!(options.timeout_secs, 9);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parse_tui_and_bare_refact() {
        assert!(matches!(
            parse_from(["refact"]).unwrap(),
            RefactCliCommand::Tui(TuiOptions { project: None })
        ));
        let command = parse_from(["refact", "tui", "--project", "/tmp/project"]).unwrap();
        match command {
            RefactCliCommand::Tui(options) => {
                assert_eq!(
                    options.project,
                    Some(std::path::PathBuf::from("/tmp/project"))
                );
            }
            _ => panic!("expected tui command"),
        }
    }

    #[test]
    fn dispatch_daemon_command() {
        assert!(matches!(
            dispatch(RefactCliCommand::Daemon { foreground: false }),
            DispatchResult::Daemon { foreground: false }
        ));
    }

    #[test]
    fn parse_unknown_subcommand_errors() {
        let error = parse_from(["refact", "bogus"]).unwrap_err();
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("unknown subcommand `bogus`"));
    }

    #[test]
    fn json_usage_errors_are_marked_for_machine_output() {
        let run = parse_from(["refact", "run", "--json"]).unwrap_err();
        assert_eq!(run.exit_code, 2);
        assert!(run.json);
        assert_eq!(run.kind, Some("usage"));

        let control = parse_from(["refact", "logs", "--json", "-f"]).unwrap_err();
        assert_eq!(control.exit_code, 2);
        assert!(control.json);
        assert_eq!(control.kind, Some("usage"));
    }

    #[test]
    fn subcommand_daemon_help_routes_to_scoped_text() {
        match parse_from(["refact", "logs", "--help"]).unwrap() {
            RefactCliCommand::Help(text) => {
                assert!(text.starts_with("refact logs"));
                assert!(!text.contains("SUBCOMMANDS:"));
            }
            _ => panic!("expected help command"),
        }

        match parse_from(["refact", "run", "--help"]).unwrap() {
            RefactCliCommand::Help(text) => {
                assert!(text.starts_with("refact run"));
                assert!(text.contains("--approve"));
            }
            _ => panic!("expected help command"),
        }
    }

    #[test]
    fn parse_tui_rejects_unknown_argument() {
        let error = parse_from(["refact", "tui", "--bogus"]).unwrap_err();
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("unexpected tui argument"));
    }
}
