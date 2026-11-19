use refact_python_kernel::{client::*, protocol::*};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
async fn event(c: &mut KernelClient) -> ClientEvent {
    tokio::time::timeout(Duration::from_secs(10), c.next_event())
        .await
        .unwrap()
}
fn command() -> WorkerCommand {
    WorkerCommand {
        program: env!("CARGO_BIN_EXE_refact-python-worker").into(),
        args: vec![],
        env: HashMap::new(),
        cwd: None,
    }
}
async fn client() -> KernelClient {
    let t = Instant::now();
    let mut c = KernelClient::spawn(command(), ClientOptions::default()).unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Ready { .. })
    ));
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(c.pid().unwrap());
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing().with_memory(),
    );
    eprintln!(
        "worker ready {:?}, idle RSS {} bytes",
        t.elapsed(),
        sys.process(pid).unwrap().memory()
    );
    c
}
async fn feed(c: &KernelClient, code: &str) {
    c.send(KernelCommand::Feed {
        run_id: "test".into(),
        code: code.into(),
        globals: vec![],
        limits: RunLimits {
            max_compute_ms: 60_000,
            ..RunLimits::default()
        },
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn worker_process_round_trip() {
    let mut c = client().await;
    feed(&c, "42").await;
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==42)
    );
    c.send(KernelCommand::Shutdown).await.unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Exited {
            code: Some(0),
            ..
        }
    ));
}
#[tokio::test]
async fn killing_worker_mid_compute_reports_exit() {
    let mut c = client().await;
    feed(&c, "while True: pass").await;
    c.kill().await.unwrap();
    assert!(matches!(event(&mut c).await, ClientEvent::Exited { .. }));
}
#[tokio::test]
async fn memory_watchdog_kills_runaway_small_allocations() {
    let mut c = client().await;
    feed(&c, "items=[]\nwhile True:\n    items.append('x' * 1024)").await;
    assert!(
        matches!(event(&mut c).await,ClientEvent::MemoryExceeded{rss_bytes}if rss_bytes>128*1024*1024)
    );
    assert!(matches!(event(&mut c).await, ClientEvent::Exited { .. }));
}
#[cfg(unix)]
#[tokio::test]
async fn worker_crash_is_reported_with_stderr_tail() {
    let mut cmd = command();
    cmd.program = "/bin/sh".into();
    cmd.args = vec!["-c".into(), "echo worker-crash >&2; exit 17".into()];
    let mut c = KernelClient::spawn(cmd, ClientOptions::default()).unwrap();
    assert!(
        matches!(event(&mut c).await,ClientEvent::Exited{code:Some(17),stderr_tail}if stderr_tail.contains("worker-crash"))
    );
}
#[cfg(unix)]
#[tokio::test]
async fn worker_does_not_depend_on_main_thread_stack() {
    let mut cmd = command();
    cmd.args = vec![
        "-c".into(),
        "ulimit -s 256 && exec \"$0\" \"$@\"".into(),
        cmd.program.to_string_lossy().into_owned(),
    ];
    cmd.program = "/bin/sh".into();
    let mut c = KernelClient::spawn(cmd, ClientOptions::default()).unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Ready { .. })
    ));
    feed(&c, "a=0\nfor i in range(150): a=[a]\nrepr(a)").await;
    let e = event(&mut c).await;
    assert!(
        matches!(e, ClientEvent::Kernel(KernelEvent::Completed { .. })),
        "{e:?}"
    );
}
#[cfg(unix)]
#[test]
fn worker_environment_does_not_inherit_parent_sentinel() {
    const SENTINEL: &str = "REFACT_KERNEL_PARENT_SECRET_SENTINEL";
    if std::env::var_os(SENTINEL).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "worker_environment_does_not_inherit_parent_sentinel",
                "--nocapture",
            ])
            .env(SENTINEL, "parent-secret")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "sentinel subprocess failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut c = KernelClient::spawn(
            WorkerCommand {
                program: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "test -z \"${REFACT_KERNEL_PARENT_SECRET_SENTINEL+x}\" && test \"$REFACT_KERNEL_EXPLICIT_ENV\" = explicit".into(),
                ],
                env: HashMap::from([("REFACT_KERNEL_EXPLICIT_ENV".into(), "explicit".into())]),
                cwd: None,
            },
            ClientOptions::default(),
        )
        .unwrap();
        assert!(matches!(
            event(&mut c).await,
            ClientEvent::Exited { code: Some(0), .. }
        ));
    });
}
#[tokio::test]
async fn snapshot_file_survives_worker_restart() {
    let path = std::env::temp_dir()
        .join(format!("worker-test-{}.snapshot", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    let mut c = client().await;
    feed(&c, "answer=42").await;
    event(&mut c).await;
    c.send(KernelCommand::Snapshot {
        path: path.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::SnapshotDone { .. })
    ));
    c.send(KernelCommand::Shutdown).await.unwrap();
    event(&mut c).await;
    let mut c = client().await;
    c.send(KernelCommand::LoadSnapshot {
        max_state_bytes: 1024 * 1024 * 1024,
        expected_sha256: {
            use sha2::Digest;
            sha2::Sha256::digest(std::fs::read(&path).unwrap())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        },
        path: path.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Loaded)
    ));
    feed(&c, "answer").await;
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==42)
    );
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn dropping_client_kills_worker() {
    let c = client().await;
    let pid = sysinfo::Pid::from_u32(c.pid().unwrap());
    drop(c);
    tokio::time::timeout(Duration::from_secs(5), async move {
        let mut sys = sysinfo::System::new();
        loop {
            sys.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&[pid]),
                true,
                sysinfo::ProcessRefreshKind::nothing().with_memory(),
            );
            if sys.process(pid).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn worker_serves_file_io_and_reports_fs_events() {
    let root = std::env::temp_dir().join(format!("hostfs-worker-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut c = client().await;
    c.send(KernelCommand::Configure {
        project_root: root.clone(),
        env: Default::default(),
        capture_dir: Some(root.join("captures")),
        deny_write: Default::default(),
        deny_read: Default::default(),
    })
    .await
    .unwrap();
    feed(&c, "from pathlib import Path\nPath('sample').write_text('hello')\nwith open('sample') as f:\n    result = f.read()\nresult").await;
    let (mut fs, mut capture) = (false, false);
    loop {
        match event(&mut c).await {
            ClientEvent::Kernel(KernelEvent::Fs {
                path, ..
            }) => {
                assert!(std::path::Path::new(&path).is_absolute());
                fs = true;
            }
            ClientEvent::Kernel(KernelEvent::WriteCapture {
                ..
            }) => capture = true,
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => {
                assert_eq!(value, "hello");
                break;
            }
            e => panic!("unexpected {e:?}"),
        }
    }
    assert!(fs && capture);
    c.send(KernelCommand::Shutdown).await.unwrap();
    let _ = std::fs::remove_dir_all(root);
}
