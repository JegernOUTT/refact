fn main() {
    refact_lsp::self_exec::run_if_requested();
    let cmdline = refact_lsp::global_context::CommandLine::from_args();
    let runtime = refact_lsp::runtime_config::build_tokio_runtime();
    runtime.block_on(refact_lsp::run_with_cmdline(cmdline));
}
