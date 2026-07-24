const DEFAULT_TOKIO_WORKER_STACK_MB: usize = 16;
const MIN_TOKIO_WORKER_STACK_MB: usize = 2;
const MAX_TOKIO_WORKER_STACK_MB: usize = 256;

fn tokio_worker_stack_bytes() -> usize {
    std::env::var("REFACT_TOKIO_WORKER_STACK_MB")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TOKIO_WORKER_STACK_MB)
        .clamp(MIN_TOKIO_WORKER_STACK_MB, MAX_TOKIO_WORKER_STACK_MB)
        * 1024
        * 1024
}

fn main() {
    let command = refact_lsp::cli_dispatch::parse_from_env().unwrap_or_else(|error| error.exit());
    match refact_lsp::cli_dispatch::dispatch(command) {
        refact_lsp::cli_dispatch::DispatchResult::Worker(cmdline) => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all();
            builder.thread_stack_size(tokio_worker_stack_bytes());
            let runtime = builder.build().expect("failed to build tokio runtime");
            runtime.block_on(refact_lsp::run_with_cmdline(cmdline));
        }
        refact_lsp::cli_dispatch::DispatchResult::Daemon { foreground, port } => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all();
            builder.thread_stack_size(tokio_worker_stack_bytes());
            let runtime = builder.build().expect("failed to build tokio runtime");
            runtime.block_on(refact_lsp::daemon::run_daemon(foreground, port));
        }
        refact_lsp::cli_dispatch::DispatchResult::Run(options) => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all();
            builder.thread_stack_size(tokio_worker_stack_bytes());
            let runtime = builder.build().expect("failed to build tokio runtime");
            let mut io = refact_lsp::daemon::run_cmd::StdRunIo;
            let code = runtime.block_on(refact_lsp::daemon::run_cmd::run(options, &mut io));
            std::process::exit(code);
        }
        refact_lsp::cli_dispatch::DispatchResult::Tui(options) => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all();
            builder.thread_stack_size(tokio_worker_stack_bytes());
            let runtime = builder.build().expect("failed to build tokio runtime");
            let daemon = runtime
                .block_on(refact_lsp::daemon::client::ensure_daemon_running())
                .unwrap_or_else(|error| {
                    eprintln!("daemon unreachable: {error}");
                    std::process::exit(1);
                });
            let daemon_url = Some(refact_lsp::daemon::chat_client::daemon_base_url(&daemon));
            let result = runtime.block_on(refact_tui::run_tui(daemon_url, options.project));
            if let Err(error) = result {
                eprintln!("refact tui failed: {error}");
                std::process::exit(1);
            }
        }
        refact_lsp::cli_dispatch::DispatchResult::Control(options) => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all();
            builder.thread_stack_size(tokio_worker_stack_bytes());
            let runtime = builder.build().expect("failed to build tokio runtime");
            let code = runtime.block_on(refact_lsp::daemon::cli::run(options));
            std::process::exit(code);
        }
        refact_lsp::cli_dispatch::DispatchResult::SelfUpdate(options) => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all();
            builder.thread_stack_size(tokio_worker_stack_bytes());
            let runtime = builder.build().expect("failed to build tokio runtime");
            let code = runtime.block_on(refact_lsp::self_update::run(options));
            std::process::exit(code);
        }
        refact_lsp::cli_dispatch::DispatchResult::Exit(code) => std::process::exit(code),
    }
}
