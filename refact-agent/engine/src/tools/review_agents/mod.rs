pub mod config;
pub mod contract;
pub mod prompts;
pub mod runner;
pub mod stages;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use tokio::sync::Semaphore;

use crate::global_context::GlobalContext;
use crate::tools::review_agents::runner::{
    monitor_ctx, now_ms, run_stage, StageCtx, StageJob, StageProduct,
};
use crate::tools::review_types::StageRun;

pub const WATCHDOG_POLL: Duration = Duration::from_secs(5);
pub const MIN_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const HARVEST_GRACE: Duration = Duration::from_secs(10);

type BoxedProduct = Pin<Box<dyn Future<Output = StageProduct> + Send>>;

#[async_trait]
pub trait StageExecutor: Send + Sync {
    async fn execute(&self, job: StageJob, activity: Arc<AtomicU64>) -> StageProduct;

    fn abort_flag(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}

pub struct SubchatExecutor {
    pub gcx: Arc<GlobalContext>,
    pub ctx: StageCtx,
}

#[async_trait]
impl StageExecutor for SubchatExecutor {
    async fn execute(&self, job: StageJob, activity: Arc<AtomicU64>) -> StageProduct {
        let (ctx, forwarder) = monitor_ctx(&self.ctx, activity);
        let product = run_stage(self.gcx.clone(), ctx, job).await;
        forwarder.abort();
        product
    }

    fn abort_flag(&self) -> Option<Arc<AtomicBool>> {
        Some(self.ctx.abort_flag.clone())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScheduleParams {
    pub parallel_depth: usize,
    pub idle_timeout: Duration,
    pub grace: Duration,
}

impl ScheduleParams {
    fn idle_limit(&self) -> Duration {
        self.idle_timeout.max(MIN_IDLE_TIMEOUT)
    }
}

fn stage_product(run: StageRun) -> StageProduct {
    StageProduct {
        run,
        findings: vec![],
        verdicts: vec![],
        metering: serde_json::Map::new(),
        raw: None,
    }
}

fn idle_timed_out(label: &str, trace: &str, idle_ms: u64, started: u64) -> StageProduct {
    stage_product(
        StageRun::timed_out(
            label,
            None,
            now_ms().saturating_sub(started),
            &format!(
                "no activity for {}s; the stage stopped responding",
                idle_ms / 1000
            ),
        )
        .with_trace_chat_id(Some(trace.to_string())),
    )
}

fn cancelled_before_start(label: &str) -> StageProduct {
    stage_product(StageRun::not_run(
        label,
        "review cancelled before the stage started",
    ))
}

fn panicked(label: &str, trace: &str, started: u64) -> StageProduct {
    stage_product(
        StageRun::failed(
            label,
            None,
            now_ms().saturating_sub(started),
            "stage panicked",
        )
        .with_trace_chat_id(Some(trace.to_string())),
    )
}

fn salvaged(mut product: StageProduct, kill: StageRun) -> StageProduct {
    product.run.status = kill.status;
    product.run.reason = match (kill.reason, product.run.reason.take()) {
        (Some(kill_reason), Some(original)) if original != kill_reason => {
            Some(format!("{kill_reason}; {original}"))
        }
        (kill_reason, original) => kill_reason.or(original),
    };
    product.run.summary = Some(match product.run.summary.take() {
        Some(summary) => format!("{summary}; salvaged from partial run"),
        None => "salvaged from partial run".to_string(),
    });
    product
}

#[allow(clippy::too_many_arguments)]
async fn watch_stage(
    label: String,
    trace: String,
    idle_limit: Duration,
    poll: Duration,
    grace: Duration,
    activity: Arc<AtomicU64>,
    abort: Arc<AtomicBool>,
    parent_abort: Option<Arc<AtomicBool>>,
    work: BoxedProduct,
) -> StageProduct {
    let started = now_ms();
    activity.store(started, Ordering::Relaxed);
    let idle_limit_ms = idle_limit.as_millis() as u64;
    let poll = poll.max(Duration::from_millis(1));
    tokio::pin!(work);
    let fallback = loop {
        tokio::select! {
            product = &mut work => {
                abort.store(true, Ordering::SeqCst);
                return product;
            }
            _ = tokio::time::sleep(poll) => {
                if parent_abort
                    .as_ref()
                    .is_some_and(|parent| parent.load(Ordering::SeqCst))
                {
                    abort.store(true, Ordering::SeqCst);
                }
                let idle_ms = now_ms().saturating_sub(activity.load(Ordering::Relaxed));
                if idle_ms > idle_limit_ms {
                    break idle_timed_out(&label, &trace, idle_ms, started);
                }
            }
        }
    };
    abort.store(true, Ordering::SeqCst);
    match tokio::time::timeout(grace.max(Duration::from_millis(1)), &mut work).await {
        Ok(product) => salvaged(product, fallback.run),
        Err(_) => fallback,
    }
}

pub async fn run_stage_jobs(
    executor: Arc<dyn StageExecutor>,
    jobs: Vec<StageJob>,
    params: ScheduleParams,
) -> Vec<StageProduct> {
    let semaphore = Arc::new(Semaphore::new(params.parallel_depth.max(1)));
    let idle_limit = params.idle_limit();
    let poll = WATCHDOG_POLL
        .min(idle_limit / 2)
        .max(Duration::from_millis(1));
    let parent_abort = executor.abort_flag();
    let futures = jobs.into_iter().map(|job| {
        let executor = executor.clone();
        let semaphore = semaphore.clone();
        let parent_abort = parent_abort.clone();
        async move {
            let label = job.label.clone();
            let trace = job.trace_chat_id.clone();
            let abort = job.abort.clone();
            let Ok(_permit) = semaphore.acquire_owned().await else {
                abort.store(true, Ordering::SeqCst);
                return cancelled_before_start(&label);
            };
            if parent_abort
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
            {
                abort.store(true, Ordering::SeqCst);
                return cancelled_before_start(&label);
            }
            let activity = Arc::new(AtomicU64::new(now_ms()));
            let panic_label = label.clone();
            let panic_trace = trace.clone();
            let stamp = activity.clone();
            let started = now_ms();
            let work: BoxedProduct = Box::pin(async move {
                match std::panic::AssertUnwindSafe(executor.execute(job, stamp))
                    .catch_unwind()
                    .await
                {
                    Ok(product) => product,
                    Err(_) => panicked(&panic_label, &panic_trace, started),
                }
            });
            watch_stage(
                label,
                trace,
                idle_limit,
                poll,
                params.grace,
                activity,
                abort,
                parent_abort,
                work,
            )
            .await
        }
    });
    futures::future::join_all(futures).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::call_validation::{ChatModelType, SubchatParameters};
    use crate::llm::params::CacheControl;
    use crate::subchat::ExplicitSubchatSpec;
    use crate::tools::review_agents::runner::stage_trace_chat_id;
    use crate::tools::review_agents::stages::embedded_catalog;
    use crate::tools::review_types::StageStatusKind;

    fn spec_for(stage_id: &str) -> Arc<crate::tools::review_agents::stages::StageSpec> {
        Arc::new(
            embedded_catalog()
                .into_iter()
                .find(|spec| spec.id == stage_id)
                .expect("stage id is present in the embedded catalog"),
        )
    }

    fn job(label: &str, stage_id: &str) -> StageJob {
        StageJob {
            spec: spec_for(stage_id),
            label: label.to_string(),
            subchat: ExplicitSubchatSpec {
                params: SubchatParameters {
                    subchat_model_type: ChatModelType::Default,
                    subchat_model: "m".to_string(),
                    subchat_n_ctx: 100000,
                    subchat_max_new_tokens: 8000,
                    subchat_temperature: None,
                    subchat_tokens_for_rag: 0,
                    subchat_reasoning_effort: None,
                    subchat_cache_control: CacheControl::Off,
                },
                model: "m".to_string(),
                autonomous_no_confirm: true,
            },
            max_steps: 5,
            prompt: String::new(),
            trace_chat_id: stage_trace_chat_id("rv-1", label),
            abort: Arc::new(AtomicBool::new(false)),
        }
    }

    fn params(parallel_depth: usize) -> ScheduleParams {
        ScheduleParams {
            parallel_depth,
            idle_timeout: Duration::from_secs(120),
            grace: Duration::from_millis(40),
        }
    }

    struct CountingExecutor {
        active: AtomicUsize,
        peak: AtomicUsize,
        delay: Duration,
    }

    #[async_trait]
    impl StageExecutor for CountingExecutor {
        async fn execute(&self, job: StageJob, _activity: Arc<AtomicU64>) -> StageProduct {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            StageProduct {
                run: StageRun::ok(&job.label, Some("m".to_string()), 1),
                findings: vec![],
                verdicts: vec![],
                metering: serde_json::Map::new(),
                raw: None,
            }
        }
    }

    struct PanickingExecutor;

    #[async_trait]
    impl StageExecutor for PanickingExecutor {
        async fn execute(&self, _job: StageJob, _activity: Arc<AtomicU64>) -> StageProduct {
            panic!("stage exploded");
        }
    }

    fn never_finishing() -> BoxedProduct {
        Box::pin(async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            unreachable!()
        })
    }

    #[tokio::test]
    async fn review_scheduler_never_exceeds_parallel_depth() {
        let executor = Arc::new(CountingExecutor {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay: Duration::from_millis(30),
        });
        let jobs: Vec<StageJob> = [
            "mechanical",
            "diff",
            "impact",
            "spec",
            "security",
            "simplicity",
        ]
        .iter()
        .map(|id| job(id, id))
        .collect();

        let products = run_stage_jobs(executor.clone(), jobs, params(2)).await;

        assert_eq!(products.len(), 6);
        assert!(products
            .iter()
            .all(|product| product.run.status == StageStatusKind::Ok));
        assert!(executor.peak.load(Ordering::SeqCst) <= 2);
        assert!(executor.peak.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn review_scheduler_turns_a_panicking_stage_into_a_failed_row() {
        let products = run_stage_jobs(
            Arc::new(PanickingExecutor),
            vec![job("spec", "spec")],
            params(2),
        )
        .await;

        assert_eq!(products[0].run.status, StageStatusKind::Failed);
        assert_eq!(products[0].run.reason.as_deref(), Some("stage panicked"));
        assert_eq!(
            products[0].run.trace_chat_id.as_deref(),
            Some("subchat-rv-1-spec")
        );
    }

    #[test]
    fn review_scheduler_idle_limit_never_drops_below_the_floor() {
        let mut schedule = params(1);
        schedule.idle_timeout = Duration::from_secs(1);
        assert_eq!(schedule.idle_limit(), MIN_IDLE_TIMEOUT);

        schedule.idle_timeout = Duration::from_secs(300);
        assert_eq!(schedule.idle_limit(), Duration::from_secs(300));
    }

    #[tokio::test]
    async fn review_watchdog_kills_a_silent_stage() {
        let activity = Arc::new(AtomicU64::new(now_ms()));
        let abort = Arc::new(AtomicBool::new(false));

        let product = watch_stage(
            "diff".to_string(),
            "subchat-rv-1-diff".to_string(),
            Duration::from_millis(60),
            Duration::from_millis(10),
            Duration::from_millis(40),
            activity,
            abort.clone(),
            None,
            never_finishing(),
        )
        .await;

        assert_eq!(product.run.status, StageStatusKind::TimedOut);
        assert!(abort.load(Ordering::SeqCst));
        assert!(product
            .run
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("stopped responding")));
        assert_eq!(
            product.run.trace_chat_id.as_deref(),
            Some("subchat-rv-1-diff")
        );
        assert!(product.findings.is_empty());
    }

    async fn assert_heartbeating_stage_survives(
        iterations: usize,
        beat: Duration,
        idle_limit: Duration,
        poll: Duration,
    ) -> StageProduct {
        let activity = Arc::new(AtomicU64::new(now_ms()));
        let abort = Arc::new(AtomicBool::new(false));
        let heartbeat = activity.clone();
        let work: BoxedProduct = Box::pin(async move {
            let started = now_ms();
            for _ in 0..iterations {
                tokio::time::sleep(beat).await;
                heartbeat.store(now_ms(), Ordering::Relaxed);
            }
            StageProduct {
                run: StageRun::ok(
                    "diff",
                    Some("m".to_string()),
                    now_ms().saturating_sub(started),
                ),
                findings: vec![],
                verdicts: vec![],
                metering: serde_json::Map::new(),
                raw: None,
            }
        });

        let product = watch_stage(
            "diff".to_string(),
            "subchat-rv-1-diff".to_string(),
            idle_limit,
            poll,
            Duration::from_millis(40),
            activity,
            abort.clone(),
            None,
            work,
        )
        .await;

        assert_eq!(product.run.status, StageStatusKind::Ok);
        assert!(product.run.reason.is_none());
        assert!(abort.load(Ordering::SeqCst));
        product
    }

    #[tokio::test]
    async fn review_watchdog_keeps_an_actively_working_stage_alive_past_the_idle_limit() {
        assert_heartbeating_stage_survives(
            10,
            Duration::from_millis(30),
            Duration::from_millis(60),
            Duration::from_millis(10),
        )
        .await;
    }

    #[tokio::test]
    async fn review_watchdog_never_kills_a_stage_that_stays_active_indefinitely() {
        let idle_limit = Duration::from_millis(40);
        let product = assert_heartbeating_stage_survives(
            60,
            Duration::from_millis(10),
            idle_limit,
            Duration::from_millis(5),
        )
        .await;
        assert!(product.run.duration_ms >= 10 * idle_limit.as_millis() as u64);
    }

    #[tokio::test]
    async fn review_watchdog_streaming_stamps_keep_a_silent_channel_stage_alive_until_the_stream_stops(
    ) {
        let activity = Arc::new(AtomicU64::new(now_ms()));
        let abort = Arc::new(AtomicBool::new(false));
        let streaming_stamp = activity.clone();
        let stage_abort = abort.clone();
        let idle_limit = Duration::from_millis(60);
        let salvage_job = job("diff", "diff");
        let work: BoxedProduct = Box::pin(async move {
            let started = now_ms();
            while now_ms().saturating_sub(started) < 5 * idle_limit.as_millis() as u64 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                streaming_stamp.store(now_ms(), Ordering::Relaxed);
            }
            while !stage_abort.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let transcript = r#"{"findings":[{"title":"t","severity":"high","file":"src/lib.rs","line_start":4,"line_end":6,"claim":"c","evidence":"e","reproduction":null,"fix":null}],"summary":"partial","coverage":{"files_read":["src/lib.rs"],"commands_run":[]}}"#;
            let output = crate::tools::review_agents::contract::parse_stage_output(transcript)
                .expect("the salvage transcript carries a valid contract block");
            crate::tools::review_agents::runner::findings_product(
                "diff",
                &salvage_job,
                output,
                "m".to_string(),
                now_ms().saturating_sub(started),
            )
        });

        let product = watch_stage(
            "diff".to_string(),
            "subchat-rv-1-diff".to_string(),
            idle_limit,
            Duration::from_millis(5),
            Duration::from_secs(2),
            activity,
            abort.clone(),
            None,
            work,
        )
        .await;

        assert!(product.run.duration_ms >= 5 * idle_limit.as_millis() as u64);
        assert_eq!(product.run.status, StageStatusKind::TimedOut);
        assert!(product
            .run
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("stopped responding")));
        assert_eq!(product.findings.len(), 1);
        assert!(product
            .run
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("salvaged from partial run")));
        assert!(abort.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn review_watchdog_salvages_findings_from_a_soft_aborted_stage() {
        let activity = Arc::new(AtomicU64::new(now_ms()));
        let abort = Arc::new(AtomicBool::new(false));
        let stage_abort = abort.clone();
        let salvage_job = job("diff", "diff");
        let work: BoxedProduct = Box::pin(async move {
            while !stage_abort.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let transcript = r#"{"findings":[{"title":"t","severity":"high","file":"src/lib.rs","line_start":4,"line_end":6,"claim":"c","evidence":"e","reproduction":null,"fix":null}],"summary":"partial","coverage":{"files_read":["src/lib.rs"],"commands_run":[]}}"#;
            let output = crate::tools::review_agents::contract::parse_stage_output(transcript)
                .expect("the salvage transcript carries a valid contract block");
            crate::tools::review_agents::runner::findings_product(
                "diff",
                &salvage_job,
                output,
                "m".to_string(),
                77,
            )
        });

        let product = watch_stage(
            "diff".to_string(),
            "subchat-rv-1-diff".to_string(),
            Duration::from_millis(80),
            Duration::from_millis(10),
            Duration::from_secs(2),
            activity,
            abort.clone(),
            None,
            work,
        )
        .await;

        assert_eq!(product.run.status, StageStatusKind::TimedOut);
        assert!(product
            .run
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("stopped responding")));
        assert_eq!(product.findings.len(), 1);
        assert_eq!(product.findings[0].title, "t");
        assert_eq!(product.run.findings, 1);
        assert_eq!(product.run.coverage.files_read, ["src/lib.rs"]);
        assert!(product
            .run
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("salvaged from partial run")));
        assert!(abort.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn review_watchdog_parent_abort_propagates_to_the_stage_flag() {
        let activity = Arc::new(AtomicU64::new(now_ms()));
        let abort = Arc::new(AtomicBool::new(false));
        let parent = Arc::new(AtomicBool::new(true));
        let stage_abort = abort.clone();
        let work: BoxedProduct = Box::pin(async move {
            while !stage_abort.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            StageProduct {
                run: StageRun::ok("diff", Some("m".to_string()), 5),
                findings: vec![],
                verdicts: vec![],
                metering: serde_json::Map::new(),
                raw: None,
            }
        });

        let product = watch_stage(
            "diff".to_string(),
            "subchat-rv-1-diff".to_string(),
            Duration::from_secs(600),
            Duration::from_millis(10),
            Duration::from_secs(2),
            activity,
            abort.clone(),
            Some(parent),
            work,
        )
        .await;

        assert!(abort.load(Ordering::SeqCst));
        assert_eq!(product.run.status, StageStatusKind::Ok);
    }

    #[tokio::test]
    async fn review_scheduler_leaves_no_stage_abort_flag_unset() {
        let executor = Arc::new(CountingExecutor {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay: Duration::from_millis(10),
        });
        let jobs = vec![
            job("diff", "diff"),
            job("spec", "spec"),
            job("impact", "impact"),
        ];
        let flags: Vec<Arc<AtomicBool>> = jobs.iter().map(|job| job.abort.clone()).collect();

        let products = run_stage_jobs(executor, jobs, params(1)).await;

        assert_eq!(products.len(), 3);
        assert!(products
            .iter()
            .all(|product| product.run.status == StageStatusKind::Ok));
        assert!(flags.iter().all(|flag| flag.load(Ordering::SeqCst)));
    }
}
