use refact_python_kernel::client::{ClientEvent, ClientOptions, KernelClient, WorkerCommand};
use refact_python_kernel::protocol::{KernelCommand, KernelEvent, RunLimits};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

async fn next_event(client: &mut KernelClient) -> ClientEvent {
    tokio::time::timeout(Duration::from_secs(10), client.next_event())
        .await
        .expect("python worker did not respond within ten seconds")
}

async fn evaluate(client: &mut KernelClient, run_id: &str, code: &str) -> Value {
    client
        .send(KernelCommand::Feed {
            run_id: run_id.into(),
            code: code.into(),
            globals: vec![],
            limits: RunLimits::default(),
        })
        .await
        .unwrap();
    loop {
        match next_event(client).await {
            ClientEvent::Kernel(KernelEvent::Completed {
                run_id: completed,
                value,
                truncated: false,
                ..
            }) => {
                assert_eq!(completed, run_id);
                return value;
            }
            ClientEvent::Kernel(
                KernelEvent::Fs {
                    ..
                }
                | KernelEvent::WriteCapture {
                    ..
                }
                | KernelEvent::Stdout {
                    ..
                },
            ) => {}
            event => panic!("unexpected python worker event: {event:?}"),
        }
    }
}

async fn assert_self_exec_worker_runs_code_and_host_filesystem_and_is_killed(program: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mut client = KernelClient::spawn(
        WorkerCommand {
            program: program.into(),
            args: vec!["--python-worker".into()],
            env: HashMap::new(),
            cwd: Some(dir.path().to_path_buf()),
        },
        ClientOptions {
            memory_limit_bytes: 0,
            memory_poll_ms: 250,
        },
    )
    .unwrap();
    assert!(client.pid().is_some());
    match next_event(&mut client).await {
        ClientEvent::Kernel(KernelEvent::Ready {
            ..
        }) => {}
        event => panic!("{program} --python-worker did not start the worker: {event:?}"),
    }
    client
        .send(KernelCommand::Configure {
            project_root: dir.path().to_path_buf(),
            env: Default::default(),
            capture_dir: None,
            deny_write: Default::default(),
            deny_read: Default::default(),
        })
        .await
        .unwrap();

    assert_eq!(evaluate(&mut client, "arithmetic", "1 + 1").await, json!(2));
    assert_eq!(
        evaluate(
            &mut client,
            "filesystem",
            "from pathlib import Path\np = Path('worker.txt')\np.write_text('worker content')\np.read_text()",
        )
        .await,
        json!("worker content")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("worker.txt")).unwrap(),
        "worker content"
    );

    client.kill().await.unwrap();
    assert!(matches!(
        next_event(&mut client).await,
        ClientEvent::Exited { .. }
    ));
}

#[tokio::test]
async fn python_worker_self_exec_runs_code_and_host_filesystem_and_is_killed() {
    assert_self_exec_worker_runs_code_and_host_filesystem_and_is_killed(env!(
        "CARGO_BIN_EXE_refact-lsp"
    ))
    .await;
}

#[tokio::test]
async fn python_worker_self_exec_through_refact_cli_runs_code_and_is_killed() {
    assert_self_exec_worker_runs_code_and_host_filesystem_and_is_killed(env!(
        "CARGO_BIN_EXE_refact"
    ))
    .await;
}
