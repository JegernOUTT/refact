#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use refact_exec::{ExecRegistry, ExecSpawnRequest, ExecStatus, ObservationStatus};

fn ptrace_tests_enabled() -> bool {
    std::env::var("REFACT_TEST_PTRACE").as_deref() == Ok("1")
}

fn contains_path(paths: &[PathBuf], expected: &Path) -> bool {
    paths.iter().any(|path| path == expected)
}

#[tokio::test]
#[ignore]
async fn captures_open_reads_and_writes() {
    if !ptrace_tests_enabled() {
        return;
    }
    let probe = Path::new("/tmp/rp_probe");
    let output = Path::new("/tmp/rp_out");
    std::fs::write(probe, "probe").unwrap();
    let _ = std::fs::remove_file(output);

    let result = ExecRegistry::new()
        .spawn(
            ExecSpawnRequest::foreground("sh -c 'cat /tmp/rp_probe; echo x > /tmp/rp_out'")
                .with_observe(true),
        )
        .await
        .unwrap();

    assert_eq!(
        result.snapshot.status,
        ExecStatus::Exited { exit_code: Some(0) }
    );
    let ObservationStatus::Observed(access) = result.observation else {
        panic!("Linux observer was unavailable: {:?}", result.observation);
    };
    assert!(contains_path(&access.reads, probe), "{:?}", access.reads);
    assert!(contains_path(&access.writes, output), "{:?}", access.writes);

    let _ = std::fs::remove_file(probe);
    let _ = std::fs::remove_file(output);
}

#[tokio::test]
#[ignore]
async fn abort_during_observation_leaves_no_stopped_tracee() {
    if !ptrace_tests_enabled() {
        return;
    }
    let abort_flag = Arc::new(AtomicBool::new(false));
    let abort = abort_flag.clone();
    let abort_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        abort.store(true, Ordering::Relaxed);
    });
    let registry = ExecRegistry::new();
    let marker = format!("REFACT_OBSERVE_ABORT_{}", uuid::Uuid::new_v4().simple());
    let result = registry
        .spawn(
            ExecSpawnRequest::foreground("cat /dev/zero > /dev/null")
                .with_observe(true)
                .with_env(&marker, "1")
                .with_abort_flag(abort_flag)
                .with_timeout(Duration::from_secs(10)),
        )
        .await
        .unwrap();
    abort_task.await.unwrap();

    assert_eq!(result.snapshot.status, ExecStatus::Killed);
    for _ in 0..40 {
        if processes_with_marker(&marker).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let survivors = processes_with_marker(&marker);
    for process_id in &survivors {
        let status = std::fs::read_to_string(format!("/proc/{process_id}/status")).unwrap();
        assert!(
            !status.contains("State:\tt"),
            "tracee remained stopped: {status}"
        );
    }
    panic!("tracees survived abort: {survivors:?}");
}

#[tokio::test]
#[ignore]
async fn root_exit_does_not_wait_for_background_descendant() {
    if !ptrace_tests_enabled() {
        return;
    }
    let started = Instant::now();
    let marker = format!("REFACT_OBSERVE_DESC_{}", uuid::Uuid::new_v4().simple());
    let result = ExecRegistry::new()
        .spawn(
            ExecSpawnRequest::foreground("sleep 30 &")
                .with_observe(true)
                .with_env(&marker, "1")
                .with_output_drain_timeout(Duration::from_secs(5)),
        )
        .await
        .unwrap();

    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        result.snapshot.status,
        ExecStatus::Exited { exit_code: Some(0) }
    );
    assert!(matches!(
        result.observation,
        ObservationStatus::Incomplete(_)
    ));
    for process_id in processes_with_marker(&marker) {
        unsafe {
            libc::kill(process_id as libc::pid_t, libc::SIGKILL);
        }
    }
}

#[tokio::test]
#[ignore]
async fn healthy_background_observation_is_pending() {
    if !ptrace_tests_enabled() {
        return;
    }
    let registry = ExecRegistry::new();
    let result = registry
        .spawn(ExecSpawnRequest::background("sleep 30").with_observe(true))
        .await
        .unwrap();
    let process_id = result.snapshot.meta.process_id;

    assert!(matches!(result.observation, ObservationStatus::Pending(_)));
    let reader = registry.observation_reader(&process_id).await.unwrap();
    assert!(matches!(reader.status(), ObservationStatus::Pending(_)));

    registry.kill(&process_id).await.unwrap();
}

fn processes_with_marker(marker: &str) -> Vec<u32> {
    let expected = format!("{marker}=1").into_bytes();
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let process_id = entry.file_name().to_str()?.parse::<u32>().ok()?;
            let environ = std::fs::read(entry.path().join("environ")).ok()?;
            environ
                .split(|byte| *byte == 0)
                .any(|variable| variable == expected)
                .then_some(process_id)
        })
        .collect()
}
