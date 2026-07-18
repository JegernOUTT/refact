use std::collections::HashMap;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, MINIMUM_CPU_UPDATE_INTERVAL};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceSample {
    pub rss_bytes: u64,
    pub cpu_percent: f32,
    pub uptime_secs: u64,
}

pub fn worker_resources(pids: &[u32]) -> HashMap<u32, ResourceSample> {
    let sysinfo_pids = pids.iter().copied().map(Pid::from_u32).collect::<Vec<_>>();
    if sysinfo_pids.is_empty() {
        return HashMap::new();
    }
    let mut system = System::new();
    let refresh_kind = ProcessRefreshKind::nothing()
        .with_memory()
        .with_cpu()
        .without_tasks();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&sysinfo_pids), true, refresh_kind);
    std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&sysinfo_pids), true, refresh_kind);
    sysinfo_pids
        .into_iter()
        .filter_map(|pid| {
            system.process(pid).map(|process| {
                (
                    pid.as_u32(),
                    ResourceSample {
                        rss_bytes: process.memory(),
                        cpu_percent: process.cpu_usage(),
                        uptime_secs: process.run_time(),
                    },
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_resources_reports_current_process() {
        let samples = worker_resources(&[std::process::id()]);
        let sample = samples.get(&std::process::id()).unwrap();
        assert!(sample.rss_bytes > 0);
        assert!(sample.cpu_percent.is_finite());
        assert!(sample.cpu_percent >= 0.0);
    }

    #[test]
    fn worker_resources_omits_dead_process() {
        assert!(worker_resources(&[u32::MAX]).is_empty());
    }

    #[test]
    fn worker_resources_returns_immediately_for_empty_pids() {
        let started = std::time::Instant::now();
        assert!(worker_resources(&[]).is_empty());
        assert!(started.elapsed() < MINIMUM_CPU_UPDATE_INTERVAL);
    }
}
