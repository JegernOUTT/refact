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
        refact_lsp::cli_dispatch::DispatchResult::Daemon { foreground } => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all();
            builder.thread_stack_size(tokio_worker_stack_bytes());
            let runtime = builder.build().expect("failed to build tokio runtime");
            runtime.block_on(refact_lsp::daemon::run_daemon(foreground));
        }
        refact_lsp::cli_dispatch::DispatchResult::Exit(code) => std::process::exit(code),
    }
}
