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
