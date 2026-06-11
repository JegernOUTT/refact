use std::ffi::OsString;
use std::io::Write;

use structopt::clap::ErrorKind;
use structopt::StructOpt;

use crate::global_context::CommandLine;

#[derive(Debug, Clone)]
pub enum RefactCliCommand {
    Worker(CommandLine),
    Daemon { foreground: bool },
    Version,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliDispatchError {
    pub message: String,
    pub exit_code: i32,
    pub use_stderr: bool,
}

#[derive(Debug, Clone)]
pub enum DispatchResult {
    Worker(CommandLine),
    Daemon { foreground: bool },
    Exit(i32),
}

impl CliDispatchError {
    pub fn exit(self) -> ! {
        if self.use_stderr {
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
        // TODO(T-16): bare `refact` will launch the TUI.
        return Ok(RefactCliCommand::Help);
    };
    match subcommand.to_string_lossy().as_ref() {
        "worker" => parse_worker(args),
        "daemon" => parse_daemon(&args),
        "version" | "--version" | "-V" => Ok(RefactCliCommand::Version),
        "help" | "--help" | "-h" => Ok(RefactCliCommand::Help),
        other => Err(usage_error(format!("unknown subcommand `{}`", other))),
    }
}

pub fn dispatch(command: RefactCliCommand) -> DispatchResult {
    match command {
        RefactCliCommand::Worker(cmdline) => DispatchResult::Worker(cmdline),
        RefactCliCommand::Daemon { foreground } => DispatchResult::Daemon { foreground },
        RefactCliCommand::Version => {
            println!("{}", version_text());
            DispatchResult::Exit(0)
        }
        RefactCliCommand::Help => {
            println!("{}", help_text());
            DispatchResult::Exit(0)
        }
    }
}

pub fn help_text() -> &'static str {
    "refact <SUBCOMMAND> [OPTIONS]\n\nUSAGE:\n    refact <SUBCOMMAND> [OPTIONS]\n\nSUBCOMMANDS:\n    worker [engine flags...]    Run the refact worker engine\n    daemon [--foreground]       Run the refact daemon\n    version                     Print version and build information\n\nRun `refact worker --help` for engine flags."
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
            "--help" | "-h" => return Ok(RefactCliCommand::Help),
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
    }
}

fn usage_error(message: String) -> CliDispatchError {
    CliDispatchError {
        message: format!("error: {}\n\n{}", message, help_text()),
        exit_code: 2,
        use_stderr: true,
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
        assert!(matches!(
            parse_from(["refact", "version"]).unwrap(),
            RefactCliCommand::Version
        ));
    }

    #[test]
    fn parse_daemon_foreground() {
        assert!(matches!(
            parse_from(["refact", "daemon", "--foreground"]).unwrap(),
            RefactCliCommand::Daemon { foreground: true }
        ));
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
    fn parse_bare_refact_returns_help() {
        assert!(matches!(
            parse_from(["refact"]).unwrap(),
            RefactCliCommand::Help
        ));
    }
}
