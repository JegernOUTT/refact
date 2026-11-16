fn main() {
    if let Err(e) = refact_python_kernel::worker::run_worker_stdio() {
        eprintln!("python worker: {e}");
        std::process::exit(1);
    }
}
