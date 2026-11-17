use std::ffi::OsStr;

pub const PYTHON_WORKER_ARG: &str = "--python-worker";

pub fn run_if_requested() {
    if std::env::args_os().nth(1).as_deref() == Some(OsStr::new(PYTHON_WORKER_ARG)) {
        let status = match refact_python_kernel::worker::run_worker_stdio() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("{error}");
                1
            }
        };
        std::process::exit(status);
    }
    if let Some(result) = refact_sandbox::run_sandbox_exec_from_env() {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(refact_sandbox::SANDBOX_LAUNCHER_FAILURE_EXIT_CODE);
        }
        unreachable!();
    }
}
