use structopt::StructOpt;

fn main() {
    if let Some(result) = refact_sandbox::run_sandbox_exec_from_env() {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(refact_sandbox::SANDBOX_LAUNCHER_FAILURE_EXIT_CODE);
        }
        unreachable!();
    }
    let cmdline = refact_lsp::global_context::CommandLine::from_args();
    let runtime = refact_lsp::runtime_config::build_tokio_runtime();
    runtime.block_on(refact_lsp::run_with_cmdline(cmdline));
}
