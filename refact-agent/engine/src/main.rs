const MIN_TOKIO_WORKER_STACK_MB: usize = 2;
const MAX_TOKIO_WORKER_STACK_MB: usize = 256;

fn configured_tokio_worker_stack_bytes() -> Option<usize> {
    std::env::var("REFACT_TOKIO_WORKER_STACK_MB")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|mb| mb.clamp(MIN_TOKIO_WORKER_STACK_MB, MAX_TOKIO_WORKER_STACK_MB) * 1024 * 1024)
}

fn main() {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    if let Some(stack_bytes) = configured_tokio_worker_stack_bytes() {
        builder.thread_stack_size(stack_bytes);
    }
    let runtime = builder.build().expect("failed to build tokio runtime");
    runtime.block_on(refact_lsp::run());
}
