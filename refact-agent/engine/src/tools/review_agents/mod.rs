pub mod config;
pub mod contract;
pub mod prompts;
pub mod runner;
pub mod stages;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use tokio::sync::Semaphore;
use tokio::time::Instant;

use crate::global_context::GlobalContext;
use crate::tools::review_agents::runner::{now_ms, run_stage, StageCtx, StageJob, StageProduct};
use crate::tools::review_types::StageRun;

#[async_trait]
pub trait StageExecutor: Send + Sync {
    async fn execute(&self, job: StageJob) -> StageProduct;
}

pub struct SubchatExecutor {
    pub gcx: Arc<GlobalContext>,
    pub ctx: StageCtx,
}

#[async_trait]
impl StageExecutor for SubchatExecutor {
    async fn execute(&self, job: StageJob) -> StageProduct {
        run_stage(self.gcx.clone(), self.ctx.clone(), job).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScheduleParams {
    pub parallel_depth: usize,
    pub stage_budget: Duration,
    pub writes_stage_budget: Duration,
    pub deadline: Option<Instant>,
}

impl ScheduleParams {
    fn budget_for(&self, job: &StageJob) -> Duration {
        let configured = Duration::from_secs(job.spec.budget_minutes.max(1) * 60);
        let ceiling = if job.spec.writes_allowed {
            self.writes_stage_budget
        } else {
            self.stage_budget
        };
        configured.min(ceiling.max(Duration::from_secs(60)))
    }
}

fn timed_out(label: &str, budget: Duration, started: u64) -> StageProduct {
    StageProduct {
        run: StageRun::timed_out(
            label,
            None,
            now_ms().saturating_sub(started),
            &format!("stage budget of {}s exceeded", budget.as_secs()),
        ),
        findings: vec![],
        verdicts: vec![],
        metering: serde_json::Map::new(),
        raw: None,
    }
}

fn deadline_hit(label: &str) -> StageProduct {
    StageProduct {
        run: StageRun::not_run(label, "review deadline reached before the stage started"),
        findings: vec![],
        verdicts: vec![],
        metering: serde_json::Map::new(),
        raw: None,
    }
}

fn panicked(label: &str, started: u64) -> StageProduct {
    StageProduct {
        run: StageRun::failed(
            label,
            None,
            now_ms().saturating_sub(started),
            "stage panicked",
        ),
        findings: vec![],
        verdicts: vec![],
        metering: serde_json::Map::new(),
        raw: None,
    }
}

pub async fn run_stage_jobs(
    executor: Arc<dyn StageExecutor>,
    jobs: Vec<StageJob>,
    params: ScheduleParams,
) -> Vec<StageProduct> {
    let semaphore = Arc::new(Semaphore::new(params.parallel_depth.max(1)));
    let futures = jobs.into_iter().map(|job| {
        let executor = executor.clone();
        let semaphore = semaphore.clone();
        async move {
            let label = job.label.clone();
            let budget = params.budget_for(&job);
            let Ok(_permit) = semaphore.acquire_owned().await else {
                return deadline_hit(&label);
            };
            if params
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return deadline_hit(&label);
            }
            let started = now_ms();
            let work = std::panic::AssertUnwindSafe(executor.execute(job)).catch_unwind();
            tokio::select! {
                result = work => match result {
                    Ok(product) => product,
                    Err(_) => panicked(&label, started),
                },
                _ = tokio::time::sleep(budget) => timed_out(&label, budget, started),
                _ = async {
                    match params.deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                } => StageProduct {
                    run: StageRun::timed_out(
                        &label,
                        None,
                        now_ms().saturating_sub(started),
                        "review deadline reached while the stage was running",
                    ),
                    findings: vec![],
                    verdicts: vec![],
                    metering: serde_json::Map::new(),
                    raw: None,
                },
            }
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
    use crate::tools::review_agents::stages::embedded_catalog;
    use crate::tools::review_types::StageStatusKind;

    fn spec_for(stage_id: &str) -> Arc<crate::tools::review_agents::stages::StageSpec> {
        Arc::new(
            embedded_catalog()
                .into_iter()
                .find(|spec| spec.id == stage_id)
                .unwrap(),
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
        }
    }

    fn params(parallel_depth: usize) -> ScheduleParams {
        ScheduleParams {
            parallel_depth,
            stage_budget: Duration::from_secs(60),
            writes_stage_budget: Duration::from_secs(60),
            deadline: None,
        }
    }

    struct CountingExecutor {
        active: AtomicUsize,
        peak: AtomicUsize,
        delay: Duration,
    }

    #[async_trait]
    impl StageExecutor for CountingExecutor {
        async fn execute(&self, job: StageJob) -> StageProduct {
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

    struct HangingExecutor;

    #[async_trait]
    impl StageExecutor for HangingExecutor {
        async fn execute(&self, _job: StageJob) -> StageProduct {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            unreachable!()
        }
    }

    struct PanickingExecutor;

    #[async_trait]
    impl StageExecutor for PanickingExecutor {
        async fn execute(&self, _job: StageJob) -> StageProduct {
            panic!("stage exploded");
        }
    }

    #[tokio::test]
    async fn review_scheduler_never_exceeds_parallel_depth() {
        let executor = Arc::new(CountingExecutor {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay: Duration::from_millis(30),
        });
        let jobs: Vec<StageJob> = ["mechanical", "diff", "impact", "spec", "security", "simplicity"]
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
    async fn review_scheduler_turns_a_hung_stage_into_a_timed_out_row() {
        let mut schedule = params(4);
        schedule.stage_budget = Duration::from_millis(60);
        schedule.writes_stage_budget = Duration::from_millis(60);

        let products =
            run_stage_jobs(Arc::new(HangingExecutor), vec![job("diff", "diff")], schedule).await;

        assert_eq!(products[0].run.status, StageStatusKind::TimedOut);
        assert!(products[0]
            .run
            .reason
            .as_deref()
            .unwrap()
            .contains("stage budget"));
        assert_eq!(products[0].run.name, "diff");
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
    }

    #[tokio::test]
    async fn review_scheduler_marks_stages_not_run_once_the_review_deadline_passed() {
        let mut schedule = params(1);
        schedule.deadline = Some(Instant::now() + Duration::from_millis(40));
        let executor = Arc::new(CountingExecutor {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay: Duration::from_millis(100),
        });

        let products = run_stage_jobs(
            executor,
            vec![job("diff", "diff"), job("spec", "spec"), job("impact", "impact")],
            schedule,
        )
        .await;

        assert_eq!(products.len(), 3);
        assert!(products
            .iter()
            .any(|product| product.run.status == StageStatusKind::NotRun));
        assert!(products.iter().all(|product| {
            product.run.status != StageStatusKind::Ok || product.run.name == "diff"
        }));
    }

    #[test]
    fn review_scheduler_budget_respects_both_stage_and_global_ceilings() {
        let schedule = ScheduleParams {
            parallel_depth: 4,
            stage_budget: Duration::from_secs(300),
            writes_stage_budget: Duration::from_secs(1200),
            deadline: None,
        };

        assert_eq!(
            schedule.budget_for(&job("mechanical", "mechanical")),
            Duration::from_secs(300)
        );
        assert_eq!(
            schedule.budget_for(&job("execution", "execution")),
            Duration::from_secs(1200)
        );
        assert_eq!(
            schedule.budget_for(&job("spec", "spec")),
            Duration::from_secs(300).min(Duration::from_secs(6 * 60))
        );
    }
}
