fn main() {
    match refact_sandbox::run_sandbox_exec_from_env() {
        Some(Ok(())) => unreachable!(),
        Some(Err(error)) => {
            eprintln!("{error}");
            std::process::exit(125);
        }
        None => std::process::exit(125),
    }
}
