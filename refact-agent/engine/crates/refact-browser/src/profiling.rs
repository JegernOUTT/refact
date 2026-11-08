//! Opt-in, bounded, scalar-only browser profiling. Never accepts page data or dynamic labels.
//! Enable with REFACT_BROWSER_PROFILE=1 before process startup. Durations are real monotonic
//! wall time (not the page clock). CDP counters cover the calling thread, including nested
//! Target.sendMessageToTarget ACKs: command_us is inclusive and must not be summed with stages.
use std::sync::Mutex;
use std::time::Instant;
use headless_chrome::browser::transport::{profile_counters, profiling_enabled, ProfileCounters};
use serde::Serialize;

pub const MAX_RECORDS: usize = 8192;
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Action,
    Prechecks,
    Resolve,
    ElementState,
    Perform,
    Navigation,
    Handlers,
    Backoff,
    Report,
    Step,
}
#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub stage: Stage,
    pub elapsed_us: u64,
    pub action: Option<&'static str>,
    pub cdp: ProfileCounters,
}
#[derive(Default, Serialize)]
pub struct Snapshot {
    pub records: Vec<Record>,
    pub dropped: u64,
}
static RECORDS: Mutex<Snapshot> = Mutex::new(Snapshot {
    records: Vec::new(),
    dropped: 0,
});
/// Drains process-wide records. Intended only for isolated benchmark workers; concurrent
/// request attribution requires the caller to serialize its benchmark requests.
pub fn take() -> Snapshot {
    std::mem::take(&mut *RECORDS.lock().unwrap_or_else(|e| e.into_inner()))
}
pub struct Span {
    start: Option<(Instant, ProfileCounters)>,
    stage: Stage,
    action: Option<&'static str>,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Span {
    pub fn new(stage: Stage) -> Self {
        Self {
            start: profiling_enabled().then(|| (Instant::now(), profile_counters())),
            stage,
            action: None,
            _not_send: std::marker::PhantomData,
        }
    }
    pub fn action(action: crate::ActionKind) -> Self {
        let mut span = Self::new(Stage::Action);
        span.action = Some(action.name());
        span
    }
    pub fn measure<T>(stage: Stage, operation: impl FnOnce() -> T) -> T {
        let _span = Self::new(stage);
        operation()
    }
}
fn delta(after: ProfileCounters, before: ProfileCounters) -> ProfileCounters {
    ProfileCounters {
        commands: after.commands.saturating_sub(before.commands),
        command_us: after.command_us.saturating_sub(before.command_us),
        send_us: after.send_us.saturating_sub(before.send_us),
        target_acks: after.target_acks.saturating_sub(before.target_acks),
        evaluate: after.evaluate.saturating_sub(before.evaluate),
        call_function: after.call_function.saturating_sub(before.call_function),
        input: after.input.saturating_sub(before.input),
        frame_tree: after.frame_tree.saturating_sub(before.frame_tree),
        other: after.other.saturating_sub(before.other),
        latency_buckets: std::array::from_fn(|i| {
            after.latency_buckets[i].saturating_sub(before.latency_buckets[i])
        }),
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        let Some((start, before)) = self.start else {
            return;
        };
        let record = Record {
            stage: self.stage,
            action: self.action,
            elapsed_us: start.elapsed().as_micros().min(u64::MAX as u128) as u64,
            cdp: delta(profile_counters(), before),
        };
        let mut records = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
        push_bounded(&mut records, record);
    }
}

fn push_bounded(snapshot: &mut Snapshot, record: Record) {
    if snapshot.records.len() < MAX_RECORDS {
        snapshot.records.push(record);
    } else {
        snapshot.dropped = snapshot.dropped.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bounded_sink_preserves_first_records_and_reports_exact_overflow() {
        let mut snapshot = super::Snapshot::default();
        for n in 0..super::MAX_RECORDS + 3 {
            super::push_bounded(
                &mut snapshot,
                super::Record {
                    stage: super::Stage::Action,
                    action: Some("click"),
                    elapsed_us: n as u64,
                    cdp: Default::default(),
                },
            );
        }
        assert_eq!(snapshot.records.len(), super::MAX_RECORDS);
        assert_eq!(snapshot.dropped, 3);
        assert_eq!(
            snapshot.records.last().unwrap().elapsed_us,
            (super::MAX_RECORDS - 1) as u64
        );
        let drained = std::mem::take(&mut snapshot);
        assert_eq!(drained.dropped, 3);
        assert!(snapshot.records.is_empty());
        assert_eq!(snapshot.dropped, 0);
    }
    use super::*;
    #[test]
    fn counter_deltas_are_exact_and_do_not_underflow() {
        let before = ProfileCounters {
            commands: 4,
            command_us: 90,
            ..Default::default()
        };
        let after = ProfileCounters {
            commands: 7,
            command_us: 120,
            input: 2,
            ..Default::default()
        };
        let result = delta(after, before);
        assert_eq!(result.commands, 3);
        assert_eq!(result.command_us, 30);
        assert_eq!(result.input, 2);
        assert_eq!(delta(before, after).commands, 0);
    }
    #[test]
    fn serialized_record_contains_only_fixed_labels_and_scalar_counters() {
        let record = Record {
            stage: Stage::Perform,
            action: None,
            elapsed_us: 17,
            cdp: ProfileCounters::default(),
        };
        let value = serde_json::to_value(record).unwrap();
        assert_eq!(value["stage"], "perform");
        assert_eq!(value["elapsed_us"], 17);
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert!(value["cdp"]
            .as_object()
            .unwrap()
            .values()
            .all(|value| value.is_u64()
                || value
                    .as_array()
                    .is_some_and(|a| a.len() == 8 && a.iter().all(serde_json::Value::is_u64))));
    }
}
