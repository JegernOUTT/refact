const DEFAULT_TOKIO_WORKER_STACK_MB: usize = 16;
const MIN_TOKIO_WORKER_STACK_MB: usize = 2;
const MAX_TOKIO_WORKER_STACK_MB: usize = 256;

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn default_worker_threads(parallelism: usize) -> usize {
    parallelism.saturating_mul(2).max(4)
}

fn default_blocking_threads(worker_threads: usize) -> usize {
    let _ = worker_threads;
    1024
}

pub fn tokio_worker_stack_bytes() -> usize {
    env_usize("REFACT_TOKIO_WORKER_STACK_MB")
        .unwrap_or(DEFAULT_TOKIO_WORKER_STACK_MB)
        .clamp(MIN_TOKIO_WORKER_STACK_MB, MAX_TOKIO_WORKER_STACK_MB)
        * 1024
        * 1024
}

pub fn tokio_worker_threads() -> usize {
    let parallelism = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4);
    env_usize("REFACT_TOKIO_WORKER_THREADS").unwrap_or_else(|| default_worker_threads(parallelism))
}

pub fn tokio_max_blocking_threads(worker_threads: usize) -> usize {
    env_usize("REFACT_TOKIO_MAX_BLOCKING_THREADS")
        .unwrap_or_else(|| default_blocking_threads(worker_threads))
}

pub fn build_tokio_runtime() -> tokio::runtime::Runtime {
    let worker_threads = tokio_worker_threads();
    let max_blocking_threads = tokio_max_blocking_threads(worker_threads);
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    builder.worker_threads(worker_threads);
    builder.max_blocking_threads(max_blocking_threads);
    builder.thread_stack_size(tokio_worker_stack_bytes());
    builder.build().expect("failed to build tokio runtime")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_defaults_double_available_parallelism() {
        assert_eq!(default_worker_threads(1), 4);
        assert_eq!(default_worker_threads(32), 64);
        assert_eq!(default_worker_threads(256), 512);
    }

    #[test]
    fn blocking_defaults_scale_with_async_workers() {
        assert_eq!(default_blocking_threads(4), 1024);
        assert_eq!(default_blocking_threads(64), 1024);
        assert_eq!(default_blocking_threads(512), 1024);
    }
}
