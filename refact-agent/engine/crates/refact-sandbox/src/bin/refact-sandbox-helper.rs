fn main() {
    match refact_sandbox::run_sandbox_exec_from_env() {
        Some(Ok(())) => unreachable!(),
        Some(Err(error)) => {
            eprintln!("{error}");
            std::process::exit(refact_sandbox::SANDBOX_LAUNCHER_FAILURE_EXIT_CODE);
        }
        None => std::process::exit(refact_sandbox::SANDBOX_LAUNCHER_FAILURE_EXIT_CODE),
    }
}
