use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::Instant;
use serde::Serialize;
use tokio::sync::Mutex as AMutex;
use std::sync::Arc;

const RESPONSE_TIME_WINDOW: usize = 100;

#[derive(Clone, Serialize, Default)]
pub struct ToolCallStats {
    pub call_count: u64,
    pub error_count: u64,
    pub avg_response_ms: f64,
    pub last_called_at: Option<String>,
}

#[derive(Clone, Serialize, Default)]
pub struct MCPServerMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_memory_rss_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_cpu_percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_uptime_secs: Option<u64>,

    pub total_tool_calls: u64,
    pub successful_calls: u64,
    pub failed_calls: u64,
    pub avg_response_time_ms: f64,
    pub p95_response_time_ms: f64,
    pub max_response_time_ms: f64,

    pub tool_stats: HashMap<String, ToolCallStats>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_since: Option<String>,
    pub reconnect_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_call_at: Option<String>,
}

pub struct MCPMetricsCollector {
    pub metrics: MCPServerMetrics,
    response_window: VecDeque<f64>,
    process_start: Option<Instant>,
    last_cpu_stat: Option<(u64, Instant)>,
}

impl MCPMetricsCollector {
    pub fn new() -> Self {
        MCPMetricsCollector {
            metrics: MCPServerMetrics::default(),
            response_window: VecDeque::new(),
            process_start: None,
            last_cpu_stat: None,
        }
    }

    pub fn record_connected(&mut self) {
        self.metrics.connected_since = Some(
            chrono::Local::now()
                .format("%Y-%m-%dT%H:%M:%S%.3f")
                .to_string(),
        );
        self.process_start = Some(Instant::now());
    }

    pub fn record_reconnect(&mut self) {
        self.metrics.reconnect_count += 1;
    }

    pub fn set_pid(&mut self, pid: u32) {
        self.metrics.process_pid = Some(pid);
    }

    pub fn record_call_start(&self) -> Instant {
        Instant::now()
    }

    pub fn record_call_success(&mut self, tool_name: &str, start: Instant) {
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        self.metrics.total_tool_calls += 1;
        self.metrics.successful_calls += 1;
        let now_str = chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        self.metrics.last_call_at = Some(now_str.clone());

        self.push_response_time(elapsed_ms);
        self.update_aggregates();

        let entry = self
            .metrics
            .tool_stats
            .entry(tool_name.to_string())
            .or_default();
        let n = entry.call_count as f64;
        entry.avg_response_ms = (entry.avg_response_ms * n + elapsed_ms) / (n + 1.0);
        entry.call_count += 1;
        entry.last_called_at = Some(now_str);
    }

    pub fn record_call_failure(&mut self, tool_name: &str, start: Instant) {
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        self.metrics.total_tool_calls += 1;
        self.metrics.failed_calls += 1;
        let now_str = chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        self.metrics.last_call_at = Some(now_str.clone());

        self.push_response_time(elapsed_ms);
        self.update_aggregates();

        let entry = self
            .metrics
            .tool_stats
            .entry(tool_name.to_string())
            .or_default();
        entry.call_count += 1;
        entry.error_count += 1;
        entry.last_called_at = Some(now_str);
    }

    fn push_response_time(&mut self, ms: f64) {
        if self.response_window.len() >= RESPONSE_TIME_WINDOW {
            self.response_window.pop_front();
        }
        self.response_window.push_back(ms);
    }

    fn update_aggregates(&mut self) {
        if self.response_window.is_empty() {
            return;
        }
        let mut sorted: Vec<f64> = self.response_window.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        self.metrics.avg_response_time_ms = sorted.iter().sum::<f64>() / n as f64;
        let p95_idx = ((n as f64 * 0.95) as usize).saturating_sub(1).min(n - 1);
        self.metrics.p95_response_time_ms = sorted[p95_idx];
        self.metrics.max_response_time_ms = sorted[n - 1];
    }

    pub fn refresh_process_metrics(&mut self) {
        let pid = match self.metrics.process_pid {
            Some(p) => p,
            None => return,
        };

        #[cfg(target_os = "linux")]
        {
            if let Some(rss) = read_proc_rss(pid) {
                self.metrics.process_memory_rss_bytes = Some(rss);
            }
            if let Some(cpu) = self.sample_cpu_percent(pid) {
                self.metrics.process_cpu_percent = Some(cpu);
            }
        }

        if let Some(start) = self.process_start {
            self.metrics.process_uptime_secs = Some(start.elapsed().as_secs());
        }
    }

    #[cfg(target_os = "linux")]
    fn sample_cpu_percent(&mut self, pid: u32) -> Option<f32> {
        let stat_path = format!("/proc/{}/stat", pid);
        let content = std::fs::read_to_string(&stat_path).ok()?;
        let fields: Vec<&str> = content.split_whitespace().collect();
        if fields.len() < 15 {
            return None;
        }
        let utime: u64 = fields[13].parse().ok()?;
        let stime: u64 = fields[14].parse().ok()?;
        let total_ticks = utime + stime;
        let now = Instant::now();

        if let Some((prev_ticks, prev_time)) = self.last_cpu_stat {
            let elapsed_secs = now.duration_since(prev_time).as_secs_f64();
            if elapsed_secs > 0.0 {
                let tick_delta = total_ticks.saturating_sub(prev_ticks) as f64;
                let ticks_per_sec = get_clock_ticks_per_sec();
                let cpu_percent = (tick_delta / ticks_per_sec / elapsed_secs * 100.0) as f32;
                self.last_cpu_stat = Some((total_ticks, now));
                return Some(cpu_percent.min(100.0 * num_cpus()));
            }
        }

        self.last_cpu_stat = Some((total_ticks, now));
        None
    }

    pub fn snapshot(&mut self) -> MCPServerMetrics {
        self.refresh_process_metrics();
        self.metrics.clone()
    }
}

#[cfg(target_os = "linux")]
fn read_proc_rss(pid: u32) -> Option<u64> {
    let statm_path = format!("/proc/{}/statm", pid);
    let content = std::fs::read_to_string(&statm_path).ok()?;
    let fields: Vec<&str> = content.split_whitespace().collect();
    let rss_pages: u64 = fields.get(1)?.parse().ok()?;
    let page_size: u64 = unsafe { libc_page_size() };
    Some(rss_pages * page_size)
}

#[cfg(target_os = "linux")]
unsafe fn libc_page_size() -> u64 {
    let sz = libc::sysconf(libc::_SC_PAGESIZE);
    if sz > 0 {
        sz as u64
    } else {
        4096
    }
}

#[cfg(target_os = "linux")]
fn get_clock_ticks_per_sec() -> f64 {
    // Safety: sysconf is always safe to call with _SC_CLK_TCK
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks <= 0 {
        100.0
    } else {
        ticks as f64
    }
}

#[cfg(target_os = "linux")]
fn num_cpus() -> f32 {
    std::fs::read_to_string("/proc/cpuinfo")
        .map(|s| s.lines().filter(|l| l.starts_with("processor")).count() as f32)
        .unwrap_or(1.0)
        .max(1.0)
}

pub type SharedMetrics = Arc<AMutex<MCPMetricsCollector>>;

pub fn new_shared_metrics() -> SharedMetrics {
    Arc::new(AMutex::new(MCPMetricsCollector::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_default_zeroed() {
        let m = MCPServerMetrics::default();
        assert_eq!(m.total_tool_calls, 0);
        assert_eq!(m.successful_calls, 0);
        assert_eq!(m.failed_calls, 0);
        assert_eq!(m.reconnect_count, 0);
        assert!(m.tool_stats.is_empty());
        assert!(m.process_pid.is_none());
    }

    #[test]
    fn test_record_success_increments_counts() {
        let mut collector = MCPMetricsCollector::new();
        let start = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(5));
        collector.record_call_success("my_tool", start);
        assert_eq!(collector.metrics.total_tool_calls, 1);
        assert_eq!(collector.metrics.successful_calls, 1);
        assert_eq!(collector.metrics.failed_calls, 0);
        assert!(collector.metrics.tool_stats.contains_key("my_tool"));
        assert_eq!(collector.metrics.tool_stats["my_tool"].call_count, 1);
        assert_eq!(collector.metrics.tool_stats["my_tool"].error_count, 0);
    }

    #[test]
    fn test_record_failure_increments_counts() {
        let mut collector = MCPMetricsCollector::new();
        let start = Instant::now();
        collector.record_call_failure("bad_tool", start);
        assert_eq!(collector.metrics.total_tool_calls, 1);
        assert_eq!(collector.metrics.successful_calls, 0);
        assert_eq!(collector.metrics.failed_calls, 1);
        assert_eq!(collector.metrics.tool_stats["bad_tool"].error_count, 1);
    }

    #[test]
    fn test_p95_calculation() {
        let mut collector = MCPMetricsCollector::new();
        for i in 1..=20u64 {
            let start = Instant::now();
            collector.push_response_time(i as f64 * 10.0);
            collector.update_aggregates();
            let _ = start;
        }
        assert!(collector.metrics.p95_response_time_ms >= 180.0);
        assert!(collector.metrics.max_response_time_ms == 200.0);
    }

    #[test]
    fn test_window_capped_at_100() {
        let mut collector = MCPMetricsCollector::new();
        for i in 0..150u64 {
            collector.push_response_time(i as f64);
        }
        assert_eq!(collector.response_window.len(), 100);
        assert_eq!(collector.response_window.back().copied(), Some(149.0));
    }

    #[test]
    fn test_reconnect_count() {
        let mut collector = MCPMetricsCollector::new();
        collector.record_reconnect();
        collector.record_reconnect();
        assert_eq!(collector.metrics.reconnect_count, 2);
    }

    #[test]
    fn test_serialization_skips_none_fields() {
        let metrics = MCPServerMetrics::default();
        let json = serde_json::to_value(&metrics).unwrap();
        assert!(json.get("process_memory_rss_bytes").is_none());
        assert!(json.get("process_pid").is_none());
        assert!(json.get("connected_since").is_none());
        assert!(json.get("last_call_at").is_none());
    }

    #[test]
    fn test_multi_tool_stats() {
        let mut collector = MCPMetricsCollector::new();
        for _ in 0..3 {
            let start = Instant::now();
            collector.record_call_success("tool_a", start);
        }
        for _ in 0..2 {
            let start = Instant::now();
            collector.record_call_failure("tool_b", start);
        }
        assert_eq!(collector.metrics.tool_stats["tool_a"].call_count, 3);
        assert_eq!(collector.metrics.tool_stats["tool_b"].error_count, 2);
        assert_eq!(collector.metrics.total_tool_calls, 5);
    }
}
