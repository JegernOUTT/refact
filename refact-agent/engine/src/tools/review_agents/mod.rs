pub mod agentic;
pub mod browser_agent;
pub mod config;
pub mod dependencies;
pub mod exec_agent;
pub mod oneshot;
pub mod static_checks;
pub mod static_enrichment;
pub mod test_integrity;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{Mutex as AMutex, Semaphore};

use crate::call_validation::ChatMessage;
use crate::chat::types::TaskMeta;
use crate::global_context::GlobalContext;
use crate::tools::review_agents::config::{
    agentic_spec, enrichment_spec, oneshot_spec, slot_model_id, verifier_spec, EnrichmentSection,
    ModelSlot, OneshotSection, ReviewAgentsConfig,
};
use crate::tools::review_agents::exec_agent::ReproTarget;
use crate::tools::review_agents::oneshot::OneshotInstance;
use crate::tools::review_merge::{stable_finding_id, RiskEnrichment};
use crate::tools::review_scope::ReviewScope;
use crate::tools::review_types::{
    AgentRunReport, AgentRunStatus, ReviewDepth, ReviewFinding, ReviewSeverity,
};
use crate::tools::review_verify::{VerifierRunner, VerifyParentCtx};
use crate::worktrees::types::WorktreeMeta;

pub const A1_DEFAULT_TOOLS: &[&str] = &[
    "tree",
    "cat",
    "glob",
    "search_pattern",
    "search_symbol_definition",
    "search_semantic",
    "knowledge",
];
pub const A2_DEFAULT_TOOLS: &[&str] = &[
    "web",
    "web_search",
    "knowledge",
    "search_trajectories",
    "cat",
    "tree",
    "search_pattern",
];
pub const A3_DEFAULT_TOOLS: &[&str] = &[
    "shell",
    "cat",
    "tree",
    "glob",
    "search_pattern",
    "process_start",
    "process_read",
    "process_wait",
    "process_kill",
];
pub const A4_DEFAULT_TOOLS: &[&str] = &[
    "chrome",
    "cat",
    "tree",
    "search_pattern",
    "process_start",
    "process_read",
    "process_wait",
    "process_kill",
    "ui_probe",
    "mark_elements",
    "contrast_audit",
    "image_region",
    "visual_diff",
];

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct AgentCtx {
    pub tool_call_id: String,
    pub subchat_tx: Arc<AMutex<UnboundedSender<Value>>>,
    pub abort_flag: Arc<AtomicBool>,
    pub depth: usize,
    pub task_meta: Option<TaskMeta>,
    pub worktree: Option<WorktreeMeta>,
    pub chat_id: String,
    pub root_chat_id: String,
}

impl AgentCtx {
    pub(crate) fn verify_parent(&self) -> VerifyParentCtx {
        VerifyParentCtx {
            tool_call_id: self.tool_call_id.clone(),
            subchat_tx: self.subchat_tx.clone(),
            abort_flag: self.abort_flag.clone(),
            depth: self.depth,
            task_meta: self.task_meta.clone(),
            worktree: self.worktree.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AgentOutcome {
    pub findings: Vec<ReviewFinding>,
    pub run: AgentRunReport,
    pub metering: serde_json::Map<String, Value>,
    pub checks: Vec<String>,
    pub refuted: Vec<String>,
    pub assumed_intent: Option<String>,
    pub raw_final_answer: Option<String>,
}

impl AgentOutcome {
    fn base(agent: &str, status: AgentRunStatus, reason: Option<String>) -> Self {
        Self {
            findings: vec![],
            run: AgentRunReport {
                agent: agent.to_string(),
                model: None,
                status,
                reason,
                candidates: 0,
                survived: 0,
                duration_ms: 0,
                steps: None,
            },
            metering: serde_json::Map::new(),
            checks: vec![],
            refuted: vec![],
            assumed_intent: None,
            raw_final_answer: None,
        }
    }

    pub fn skipped(agent: &str, reason: &str) -> Self {
        Self::base(agent, AgentRunStatus::Skipped, Some(reason.to_string()))
    }

    pub fn failed(agent: &str, reason: &str, started_ms: u64) -> Self {
        let mut outcome = Self::base(agent, AgentRunStatus::Failed, Some(reason.to_string()));
        outcome.run.duration_ms = now_ms().saturating_sub(started_ms);
        outcome
    }

    pub fn ran(
        agent: &str,
        model: Option<String>,
        candidates: usize,
        findings: Vec<ReviewFinding>,
        started_ms: u64,
    ) -> Self {
        let mut outcome = Self::base(agent, AgentRunStatus::Ran, None);
        outcome.run.model = model;
        outcome.run.candidates = candidates;
        outcome.run.survived = findings.len();
        outcome.run.duration_ms = now_ms().saturating_sub(started_ms);
        outcome.findings = findings;
        outcome
    }
}

pub struct SwarmResult {
    pub findings: Vec<ReviewFinding>,
    pub agents: Vec<AgentRunReport>,
    pub metering: serde_json::Map<String, Value>,
    pub enrichment: RiskEnrichment,
    pub assumed_intent: Option<String>,
    pub refuted: Vec<String>,
    pub checks: Vec<String>,
}

struct Collector {
    findings: Vec<ReviewFinding>,
    agents: Vec<AgentRunReport>,
    metering: serde_json::Map<String, Value>,
    checks: Vec<String>,
    refuted: Vec<String>,
    assumed_intent: Option<String>,
}

impl Collector {
    fn new() -> Self {
        Self {
            findings: vec![],
            agents: vec![],
            metering: serde_json::Map::new(),
            checks: vec![],
            refuted: vec![],
            assumed_intent: None,
        }
    }

    fn absorb(&mut self, outcome: AgentOutcome) {
        self.findings.extend(outcome.findings);
        crate::tools::review_verify::merge_metering(&mut self.metering, outcome.metering);
        for check in outcome.checks {
            if !self.checks.contains(&check) {
                self.checks.push(check);
            }
        }
        self.refuted.extend(outcome.refuted);
        if self.assumed_intent.is_none() {
            self.assumed_intent = outcome.assumed_intent;
        }
        self.agents.push(outcome.run);
    }
}

type BoxedOutcome = Pin<Box<dyn Future<Output = AgentOutcome> + Send>>;

const WATCHDOG_POLL_SECS: u64 = 5;
const MIN_IDLE_TIMEOUT_SECS: u64 = 30;

pub(crate) fn monitor_ctx(
    ctx: &AgentCtx,
) -> (AgentCtx, Arc<AtomicU64>, tokio::task::JoinHandle<()>) {
    let activity = Arc::new(AtomicU64::new(now_ms()));
    let (monitor_tx, mut monitor_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let parent_tx = ctx.subchat_tx.clone();
    let stamp = activity.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(message) = monitor_rx.recv().await {
            stamp.store(now_ms(), Ordering::Relaxed);
            let sender = parent_tx.lock().await;
            let _ = sender.send(message);
        }
    });
    let mut monitored = ctx.clone();
    monitored.subchat_tx = Arc::new(AMutex::new(monitor_tx));
    (monitored, activity, forwarder)
}

fn watched(
    label: String,
    semaphore: Arc<Semaphore>,
    idle_timeout_secs: u64,
    activity: Arc<AtomicU64>,
    forwarder: tokio::task::JoinHandle<()>,
    idle_fallback: Option<AgentOutcome>,
    fut: BoxedOutcome,
) -> BoxedOutcome {
    watched_with_params(
        label,
        semaphore,
        idle_timeout_secs.max(MIN_IDLE_TIMEOUT_SECS) * 1000,
        WATCHDOG_POLL_SECS * 1000,
        activity,
        forwarder,
        idle_fallback,
        fut,
    )
}

#[allow(clippy::too_many_arguments)]
fn watched_with_params(
    label: String,
    semaphore: Arc<Semaphore>,
    idle_limit_ms: u64,
    poll_ms: u64,
    activity: Arc<AtomicU64>,
    forwarder: tokio::task::JoinHandle<()>,
    mut idle_fallback: Option<AgentOutcome>,
    fut: BoxedOutcome,
) -> BoxedOutcome {
    Box::pin(async move {
        let Ok(_permit) = semaphore.acquire_owned().await else {
            forwarder.abort();
            return AgentOutcome::skipped(&label, "semaphore_closed");
        };
        let started = now_ms();
        activity.store(started, Ordering::Relaxed);
        tokio::pin!(fut);
        let outcome = loop {
            tokio::select! {
                outcome = &mut fut => break outcome,
                _ = tokio::time::sleep(Duration::from_millis(poll_ms.max(1))) => {
                    let idle_ms = now_ms().saturating_sub(activity.load(Ordering::Relaxed));
                    if idle_ms <= idle_limit_ms {
                        continue;
                    }
                    let reason = format!("idle_timeout:no_activity_for_{}s", idle_ms / 1000);
                    break match idle_fallback.take() {
                        Some(mut outcome) => {
                            outcome.run.reason = Some(format!("enrichment_{reason}"));
                            outcome.run.duration_ms = now_ms().saturating_sub(started);
                            outcome
                        }
                        None => AgentOutcome::failed(&label, &reason, started),
                    };
                }
            }
        };
        forwarder.abort();
        outcome
    })
}

async fn build_verifier(
    gcx: Arc<GlobalContext>,
    cfg: &ReviewAgentsConfig,
) -> Result<VerifierRunner, String> {
    let system_prompt = cfg
        .swarm
        .verifier
        .prompt
        .clone()
        .filter(|p| !p.trim().is_empty())
        .ok_or("verifier prompt not configured")?;
    let model = slot_model_id(gcx, cfg.swarm.verifier.model_slot).await?;
    Ok(VerifierRunner {
        spec: verifier_spec(&cfg.swarm.verifier, model),
        system_prompt,
    })
}

fn build_repro_targets(findings: &[ReviewFinding]) -> Vec<ReproTarget> {
    let mut targets: Vec<ReproTarget> = findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.severity,
                ReviewSeverity::High | ReviewSeverity::Critical
            )
        })
        .map(|finding| ReproTarget {
            id: stable_finding_id(finding),
            file: finding.file.clone(),
            line1: finding.line1,
            line2: finding.line2,
            severity: match finding.severity {
                ReviewSeverity::Critical => "critical".to_string(),
                ReviewSeverity::High => "high".to_string(),
                ReviewSeverity::Medium => "medium".to_string(),
                ReviewSeverity::Low => "low".to_string(),
            },
            claim: finding.claim.clone(),
        })
        .collect();
    targets.dedup_by(|a, b| a.id == b.id);
    targets
}

async fn s4_outcome(scope: &ReviewScope) -> AgentOutcome {
    let started = now_ms();
    let changed: Vec<String> = scope
        .changed_files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let patch = scope.diff_patch.as_deref().unwrap_or("");
    if patch.is_empty() && changed.is_empty() {
        return AgentOutcome::skipped(test_integrity::AGENT_ID, "no_diff");
    }
    let findings = test_integrity::analyze_test_integrity(patch, &changed);
    let candidates = findings.len();
    AgentOutcome::ran(
        test_integrity::AGENT_ID,
        None,
        candidates,
        findings,
        started,
    )
}

struct OneshotPlan<'a> {
    agent_id: &'a str,
    section: &'a OneshotSection,
    prompt: Option<String>,
}

pub async fn run_review_swarm(
    gcx: Arc<GlobalContext>,
    ctx: AgentCtx,
    cfg: Arc<ReviewAgentsConfig>,
    scope: Arc<ReviewScope>,
    external_messages: Arc<Vec<ChatMessage>>,
    depth: ReviewDepth,
) -> SwarmResult {
    let mut collector = Collector::new();
    let swarm = &cfg.swarm;

    let (s6, s1, s2, s3, s4, s5) = tokio::join!(
        async {
            if swarm.s6_git_enrichment.enabled {
                static_checks::s6_enrichment(
                    gcx.clone(),
                    &scope,
                    swarm.s6_git_enrichment.max_commits,
                )
                .await
            } else {
                (
                    RiskEnrichment::default(),
                    AgentOutcome::skipped("s6_git_enrichment", "disabled").run,
                )
            }
        },
        async {
            if swarm.s1_security.enabled {
                static_checks::s1_security(gcx.clone(), &scope).await
            } else {
                AgentOutcome::skipped("s1_security", "disabled")
            }
        },
        async {
            if swarm.s2_dead_code.enabled {
                static_checks::s2_dead_code(gcx.clone(), &scope, swarm.s2_dead_code.min_confidence)
                    .await
            } else {
                AgentOutcome::skipped("s2_dead_code", "disabled")
            }
        },
        async {
            if swarm.s3_duplication.enabled {
                static_checks::s3_duplication(gcx.clone(), &scope).await
            } else {
                AgentOutcome::skipped("s3_duplication", "disabled")
            }
        },
        async {
            if swarm.s4_test_integrity.enabled {
                s4_outcome(&scope).await
            } else {
                AgentOutcome::skipped(test_integrity::AGENT_ID, "disabled")
            }
        },
        async {
            if swarm.s5_dependencies.enabled {
                dependencies::s5_dependencies(gcx.clone(), &scope).await
            } else {
                AgentOutcome::skipped(dependencies::AGENT_ID, "disabled")
            }
        },
    );
    let (enrichment, s6_row) = s6;
    collector.agents.push(s6_row);

    let verifier = match build_verifier(gcx.clone(), &cfg).await {
        Ok(runner) => Some(Arc::new(runner)),
        Err(error) => {
            tracing::warn!("review swarm: verifier unavailable: {error}");
            collector.checks.push("verifier_unavailable".to_string());
            None
        }
    };

    let semaphore = Arc::new(Semaphore::new(swarm.max_parallel.max(1)));
    let mut wave: Vec<BoxedOutcome> = vec![];

    let static_plans: Vec<(AgentOutcome, &EnrichmentSection)> = vec![
        (s1, &swarm.s1_security.agent),
        (s2, &swarm.s2_dead_code.agent),
        (s3, &swarm.s3_duplication.agent),
        (s4, &swarm.s4_test_integrity.agent),
        (s5, &swarm.s5_dependencies.agent),
    ];
    for (raw_outcome, section) in static_plans {
        if raw_outcome.findings.is_empty() || !section.enabled {
            collector.absorb(raw_outcome);
            continue;
        }
        let agent_id = raw_outcome.run.agent.clone();
        let slot = section.model_slot;
        let model = match slot_model_id(gcx.clone(), slot).await {
            Ok(model) => model,
            Err(error) => {
                let mut outcome = raw_outcome;
                outcome.run.reason = Some(format!("enrichment_skipped:{error}"));
                collector.absorb(outcome);
                continue;
            }
        };
        let system_prompt = section
            .prompt
            .clone()
            .filter(|p| !p.trim().is_empty())
            .or_else(|| {
                swarm
                    .static_enrichment_prompt
                    .clone()
                    .filter(|p| !p.trim().is_empty())
            })
            .unwrap_or_else(|| static_enrichment::DEFAULT_STATIC_ENRICHMENT_PROMPT.to_string());
        let tools = if section.tools.is_empty() {
            static_enrichment::STATIC_ENRICH_DEFAULT_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            section.tools.clone()
        };
        let label = format!("{agent_id}@{}", slot.label());
        let idle_fallback = raw_outcome.clone();
        let input = static_enrichment::StaticEnrichmentInput {
            agent_id,
            slot_label: slot.label().to_string(),
            spec: enrichment_spec(&cfg.base_params, section, model),
            system_prompt,
            tools,
            max_steps: section.max_steps.max(1),
            raw: raw_outcome,
        };
        let (agent_ctx, activity, forwarder) = monitor_ctx(&ctx);
        let fut: BoxedOutcome = Box::pin(static_enrichment::run_static_enrichment(
            gcx.clone(),
            agent_ctx,
            input,
            scope.clone(),
        ));
        wave.push(watched(
            label,
            semaphore.clone(),
            swarm.idle_timeout_secs,
            activity,
            forwarder,
            Some(idle_fallback),
            fut,
        ));
    }

    let oneshot_plans = [
        OneshotPlan {
            agent_id: "l1_diff",
            section: &swarm.l1_diff,
            prompt: swarm
                .l1_diff
                .prompt
                .clone()
                .or_else(|| Some(cfg.reviewer_prompt.clone())),
        },
        OneshotPlan {
            agent_id: "l2_simplicity",
            section: &swarm.l2_simplicity,
            prompt: swarm.l2_simplicity.prompt.clone(),
        },
        OneshotPlan {
            agent_id: "l3_spec",
            section: &swarm.l3_spec,
            prompt: swarm.l3_spec.prompt.clone(),
        },
    ];

    for plan in oneshot_plans {
        if !plan.section.enabled {
            collector.absorb(AgentOutcome::skipped(plan.agent_id, "disabled"));
            continue;
        }
        let Some(prompt) = plan.prompt.filter(|p| !p.trim().is_empty()) else {
            collector.absorb(AgentOutcome::skipped(
                plan.agent_id,
                "prompt_not_configured",
            ));
            continue;
        };
        let mut slots = plan.section.ensemble.clone();
        slots.dedup();
        if slots.is_empty() {
            slots.push(ModelSlot::Thinking);
        }
        for slot in slots {
            let label = format!("{}@{}", plan.agent_id, slot.label());
            let model = match slot_model_id(gcx.clone(), slot).await {
                Ok(model) => model,
                Err(error) => {
                    collector.absorb(AgentOutcome::skipped(&label, &error));
                    continue;
                }
            };
            let instance = OneshotInstance {
                agent_id: plan.agent_id.to_string(),
                slot_label: slot.label().to_string(),
                spec: oneshot_spec(&cfg.base_params, plan.section, model),
                instruction: prompt.clone(),
            };
            let (agent_ctx, activity, forwarder) = monitor_ctx(&ctx);
            let fut: BoxedOutcome = Box::pin(oneshot::run_oneshot_instance(
                gcx.clone(),
                agent_ctx,
                instance,
                scope.clone(),
                external_messages.clone(),
                verifier.clone(),
            ));
            wave.push(watched(
                label,
                semaphore.clone(),
                swarm.idle_timeout_secs,
                activity,
                forwarder,
                None,
                fut,
            ));
        }
    }

    let agentic_plans: Vec<(&str, &config::AgenticSection, &[&str])> = vec![
        ("a1_repo_context", &swarm.a1_repo_context, A1_DEFAULT_TOOLS),
        ("a2_research", &swarm.a2_research, A2_DEFAULT_TOOLS),
    ];
    for (agent_id, section, default_tools) in agentic_plans {
        if !section.enabled {
            collector.absorb(AgentOutcome::skipped(agent_id, "disabled"));
            continue;
        }
        let Some(prompt) = section.prompt.clone().filter(|p| !p.trim().is_empty()) else {
            collector.absorb(AgentOutcome::skipped(agent_id, "prompt_not_configured"));
            continue;
        };
        let slot = section.model_slot;
        let label = format!("{agent_id}@{}", slot.label());
        let model = match slot_model_id(gcx.clone(), slot).await {
            Ok(model) => model,
            Err(error) => {
                collector.absorb(AgentOutcome::skipped(&label, &error));
                continue;
            }
        };
        let tools = if section.tools.is_empty() {
            default_tools.iter().map(|s| s.to_string()).collect()
        } else {
            section.tools.clone()
        };
        let instance = agentic::AgenticInstance {
            agent_id: agent_id.to_string(),
            slot_label: slot.label().to_string(),
            spec: agentic_spec(&cfg.base_params, section, model),
            system_prompt: prompt,
            task_prompt: agentic::build_agent_task_prompt(&scope, None),
            tools,
            max_steps: section.max_steps.max(1),
            title: format!("Review: {agent_id}"),
            verify: true,
        };
        let (agent_ctx, activity, forwarder) = monitor_ctx(&ctx);
        let fut: BoxedOutcome = Box::pin(agentic::run_agentic_instance(
            gcx.clone(),
            agent_ctx,
            instance,
            scope.clone(),
            verifier.clone(),
        ));
        wave.push(watched(
            label,
            semaphore.clone(),
            swarm.idle_timeout_secs,
            activity,
            forwarder,
            None,
            fut,
        ));
    }

    for outcome in futures::future::join_all(wave).await {
        collector.absorb(outcome);
    }

    if depth >= ReviewDepth::Deep {
        let mut deep_wave: Vec<BoxedOutcome> = vec![];

        let exec_section = &swarm.a3_execution;
        if !exec_section.base.enabled {
            collector.absorb(AgentOutcome::skipped(exec_agent::AGENT_ID, "disabled"));
        } else if !exec_section.allow_execution {
            collector.absorb(AgentOutcome::skipped(
                exec_agent::AGENT_ID,
                "execution_disabled",
            ));
        } else if let Some(prompt) = exec_section
            .base
            .prompt
            .clone()
            .filter(|p| !p.trim().is_empty())
        {
            let slot = exec_section.base.model_slot;
            match slot_model_id(gcx.clone(), slot).await {
                Ok(model) => {
                    let tools = if exec_section.base.tools.is_empty() {
                        A3_DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect()
                    } else {
                        exec_section.base.tools.clone()
                    };
                    let input = exec_agent::ExecAgentInput {
                        slot_label: slot.label().to_string(),
                        spec: agentic_spec(&cfg.base_params, &exec_section.base, model),
                        system_prompt: prompt,
                        tools,
                        max_steps: exec_section.base.max_steps.max(1),
                        mutation_probe_cap: exec_section.mutation_probe_cap.unwrap_or(6),
                        repro_targets: build_repro_targets(&collector.findings),
                    };
                    let label = format!("{}@{}", exec_agent::AGENT_ID, slot.label());
                    let (agent_ctx, activity, forwarder) = monitor_ctx(&ctx);
                    let fut: BoxedOutcome = Box::pin(exec_agent::run_exec_agent(
                        gcx.clone(),
                        agent_ctx,
                        input,
                        scope.clone(),
                    ));
                    deep_wave.push(watched(
                        label,
                        semaphore.clone(),
                        swarm.exec_idle_timeout_secs,
                        activity,
                        forwarder,
                        None,
                        fut,
                    ));
                }
                Err(error) => {
                    collector.absorb(AgentOutcome::skipped(exec_agent::AGENT_ID, &error));
                }
            }
        } else {
            collector.absorb(AgentOutcome::skipped(
                exec_agent::AGENT_ID,
                "prompt_not_configured",
            ));
        }

        let browser_section = &swarm.a4_browser;
        if !browser_section.base.enabled {
            collector.absorb(AgentOutcome::skipped(browser_agent::AGENT_ID, "disabled"));
        } else if let Some(prompt) = browser_section
            .base
            .prompt
            .clone()
            .filter(|p| !p.trim().is_empty())
        {
            let slot = browser_section.base.model_slot;
            match slot_model_id(gcx.clone(), slot).await {
                Ok(model) => {
                    let tools = if browser_section.base.tools.is_empty() {
                        A4_DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect()
                    } else {
                        browser_section.base.tools.clone()
                    };
                    let input = browser_agent::BrowserAgentInput {
                        slot_label: slot.label().to_string(),
                        spec: agentic_spec(&cfg.base_params, &browser_section.base, model),
                        system_prompt: prompt,
                        tools,
                        max_steps: browser_section.base.max_steps.max(1),
                        section: browser_section.clone(),
                    };
                    let label = format!("{}@{}", browser_agent::AGENT_ID, slot.label());
                    let (agent_ctx, activity, forwarder) = monitor_ctx(&ctx);
                    let fut: BoxedOutcome = Box::pin(browser_agent::run_browser_agent(
                        gcx.clone(),
                        agent_ctx,
                        input,
                        scope.clone(),
                    ));
                    deep_wave.push(watched(
                        label,
                        semaphore.clone(),
                        swarm.exec_idle_timeout_secs,
                        activity,
                        forwarder,
                        None,
                        fut,
                    ));
                }
                Err(error) => {
                    collector.absorb(AgentOutcome::skipped(browser_agent::AGENT_ID, &error));
                }
            }
        } else {
            collector.absorb(AgentOutcome::skipped(
                browser_agent::AGENT_ID,
                "prompt_not_configured",
            ));
        }

        for outcome in futures::future::join_all(deep_wave).await {
            collector.absorb(outcome);
        }
    } else {
        collector.absorb(AgentOutcome::skipped(exec_agent::AGENT_ID, "depth_normal"));
        collector.absorb(AgentOutcome::skipped(
            browser_agent::AGENT_ID,
            "depth_normal",
        ));
    }

    SwarmResult {
        findings: collector.findings,
        agents: collector.agents,
        metering: collector.metering,
        enrichment,
        assumed_intent: collector.assumed_intent,
        refuted: collector.refuted,
        checks: collector.checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_types::{RankTier, ReviewEvidence, VerificationStatus};

    fn finding(severity: ReviewSeverity, file: &str, claim: &str) -> ReviewFinding {
        ReviewFinding {
            id: String::new(),
            category: "correctness".to_string(),
            severity,
            confidence: 0.8,
            verification_status: VerificationStatus::Unverified,
            rank_tier: RankTier::Unverified,
            sources: vec!["l1_diff@chat".to_string()],
            file: file.to_string(),
            line1: 1,
            line2: 2,
            claim: claim.to_string(),
            evidence: vec![ReviewEvidence {
                kind: "excerpt".to_string(),
                path: Some(file.to_string()),
                line1: Some(1),
                line2: Some(2),
                content: "excerpt".to_string(),
            }],
            impact: None,
            remediation: None,
            checks_performed: vec![],
        }
    }

    #[test]
    fn swarm_repro_targets_take_high_and_critical_only() {
        let findings = vec![
            finding(ReviewSeverity::Low, "a.rs", "low claim"),
            finding(ReviewSeverity::High, "b.rs", "high claim"),
            finding(ReviewSeverity::Critical, "c.rs", "critical claim"),
        ];
        let targets = build_repro_targets(&findings);
        assert_eq!(targets.len(), 2);
        assert!(targets.iter().all(|t| t.id.starts_with("rf-")));
        assert_eq!(targets[0].claim, "high claim");
        assert_eq!(targets[1].severity, "critical");
    }

    fn idle_test_parts() -> (Arc<Semaphore>, Arc<AtomicU64>, tokio::task::JoinHandle<()>) {
        (
            Arc::new(Semaphore::new(1)),
            Arc::new(AtomicU64::new(now_ms())),
            tokio::spawn(async {}),
        )
    }

    fn never_finishing() -> BoxedOutcome {
        Box::pin(async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            AgentOutcome::skipped("never", "unreachable")
        })
    }

    #[tokio::test]
    async fn swarm_watchdog_kills_silent_agent_without_fallback() {
        let (semaphore, activity, forwarder) = idle_test_parts();
        let outcome = watched_with_params(
            "l1_diff@chat".to_string(),
            semaphore,
            50,
            10,
            activity,
            forwarder,
            None,
            never_finishing(),
        )
        .await;
        assert_eq!(outcome.run.status, AgentRunStatus::Failed);
        assert!(outcome
            .run
            .reason
            .as_deref()
            .unwrap()
            .starts_with("idle_timeout:no_activity_for_"));
    }

    #[tokio::test]
    async fn swarm_watchdog_returns_raw_fallback_for_static_agents() {
        let (semaphore, activity, forwarder) = idle_test_parts();
        let fallback = AgentOutcome::ran(
            "s1_security",
            None,
            2,
            vec![finding(ReviewSeverity::High, "a.rs", "raw claim")],
            now_ms(),
        );
        let outcome = watched_with_params(
            "s1_security@light".to_string(),
            semaphore,
            50,
            10,
            activity,
            forwarder,
            Some(fallback),
            never_finishing(),
        )
        .await;
        assert_eq!(outcome.run.status, AgentRunStatus::Ran);
        assert_eq!(outcome.findings.len(), 1);
        assert!(outcome
            .run
            .reason
            .as_deref()
            .unwrap()
            .starts_with("enrichment_idle_timeout:"));
    }

    #[tokio::test]
    async fn swarm_watchdog_keeps_active_agent_alive_past_idle_limit() {
        let (semaphore, activity, forwarder) = idle_test_parts();
        let heartbeat = activity.clone();
        let fut: BoxedOutcome = Box::pin(async move {
            for _ in 0..10 {
                tokio::time::sleep(Duration::from_millis(30)).await;
                heartbeat.store(now_ms(), Ordering::Relaxed);
            }
            AgentOutcome::ran("l1_diff@chat", None, 1, vec![], now_ms())
        });
        let outcome = watched_with_params(
            "l1_diff@chat".to_string(),
            semaphore,
            100,
            10,
            activity,
            forwarder,
            None,
            fut,
        )
        .await;
        assert_eq!(outcome.run.status, AgentRunStatus::Ran);
        assert!(outcome.run.reason.is_none());
    }

    #[tokio::test]
    async fn swarm_monitor_ctx_forwards_messages_and_stamps_activity() {
        let (parent_tx, mut parent_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
        let ctx = AgentCtx {
            tool_call_id: "tc-1".to_string(),
            subchat_tx: Arc::new(AMutex::new(parent_tx)),
            abort_flag: Arc::new(AtomicBool::new(false)),
            depth: 0,
            task_meta: None,
            worktree: None,
            chat_id: "chat".to_string(),
            root_chat_id: "chat".to_string(),
        };
        let (monitored, activity, forwarder) = monitor_ctx(&ctx);
        activity.store(1, Ordering::Relaxed);
        monitored
            .subchat_tx
            .lock()
            .await
            .send(serde_json::json!({"subchat_id": "progress"}))
            .unwrap();
        let received = tokio::time::timeout(Duration::from_secs(2), parent_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received["subchat_id"], "progress");
        assert!(activity.load(Ordering::Relaxed) > 1);
        forwarder.abort();
    }

    #[test]
    fn swarm_collector_dedups_checks_and_takes_first_intent() {
        let mut collector = Collector::new();
        let mut first = AgentOutcome::skipped("l3_spec@chat", "x");
        first.checks = vec!["check_a".to_string()];
        first.assumed_intent = Some("intent one".to_string());
        let mut second = AgentOutcome::skipped("l3_spec@chat2", "y");
        second.checks = vec!["check_a".to_string(), "check_b".to_string()];
        second.assumed_intent = Some("intent two".to_string());
        collector.absorb(first);
        collector.absorb(second);
        assert_eq!(collector.checks, vec!["check_a", "check_b"]);
        assert_eq!(collector.assumed_intent.as_deref(), Some("intent one"));
        assert_eq!(collector.agents.len(), 2);
    }
}
