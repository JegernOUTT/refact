fn main() {
    let command = refact_lsp::cli_dispatch::parse_from_env().unwrap_or_else(|error| error.exit());
    match refact_lsp::cli_dispatch::dispatch(command) {
        refact_lsp::cli_dispatch::DispatchResult::Worker(cmdline) => {
            let runtime = refact_lsp::runtime_config::build_tokio_runtime();
            runtime.block_on(refact_lsp::run_with_cmdline(cmdline));
        }
        refact_lsp::cli_dispatch::DispatchResult::Daemon {
            foreground,
            port,
        } => {
            let runtime = refact_lsp::runtime_config::build_tokio_runtime();
            runtime.block_on(refact_lsp::daemon::run_daemon(foreground, port));
        }
        refact_lsp::cli_dispatch::DispatchResult::Run(options) => {
            let runtime = refact_lsp::runtime_config::build_tokio_runtime();
            let mut io = refact_lsp::daemon::run_cmd::StdRunIo;
            let code = runtime.block_on(refact_lsp::daemon::run_cmd::run(options, &mut io));
            std::process::exit(code);
        }
        refact_lsp::cli_dispatch::DispatchResult::Control(options) => {
            let runtime = refact_lsp::runtime_config::build_tokio_runtime();
            let code = runtime.block_on(refact_lsp::daemon::cli::run(options));
            std::process::exit(code);
        }
        refact_lsp::cli_dispatch::DispatchResult::SelfUpdate(options) => {
            let runtime = refact_lsp::runtime_config::build_tokio_runtime();
            let code = runtime.block_on(refact_lsp::self_update::run(options));
            std::process::exit(code);
        }
        refact_lsp::cli_dispatch::DispatchResult::Exit(code) => std::process::exit(code),
    }
}
