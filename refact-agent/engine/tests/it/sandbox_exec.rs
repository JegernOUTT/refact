use std::process::Command;

#[test]
fn engine_binaries_route_sandbox_exec_to_the_launcher() {
    for program in [
        env!("CARGO_BIN_EXE_refact-lsp"),
        env!("CARGO_BIN_EXE_refact"),
    ] {
        let output = Command::new(program)
            .args(["--sandbox-exec", "{}"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(refact_sandbox::SANDBOX_LAUNCHER_FAILURE_EXIT_CODE),
            "{program}: {stderr}"
        );
        assert!(
            stderr.contains("missing argv separator"),
            "{program}: {stderr}"
        );
    }
}
