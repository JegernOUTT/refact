use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::tools::review_agents::{AgentOutcome, now_ms};
use crate::tools::review_merge::{FileRiskFacts, RiskEnrichment};
use crate::tools::review_scope::ReviewScope;
use crate::tools::review_types::{
    evidence_kinds, AgentRunReport, AgentRunStatus, RankTier, ReviewEvidence, ReviewFinding,
    ReviewSeverity, VerificationStatus,
};

const S1_MAX_FILES: usize = 100;
const S1_MAX_FINDINGS: usize = 30;
const S2_MAX_FINDINGS: usize = 20;
const S3_MAX_FINDINGS: usize = 15;

fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

fn path_in_scope(candidate: &str, scope: &ReviewScope) -> Option<String> {
    let candidate_norm = normalize(candidate);
    for scoped in &scope.files {
        let scoped_str = normalize(&scoped.to_string_lossy());
        if scoped_str == candidate_norm
            || scoped_str.ends_with(&format!("/{candidate_norm}"))
            || candidate_norm.ends_with(&format!("/{scoped_str}"))
        {
            return Some(scoped_str);
        }
    }
    None
}

fn static_finding(
    agent: &str,
    category: &str,
    file: &str,
    line1: u32,
    line2: u32,
    severity: ReviewSeverity,
    confidence: f32,
    claim: String,
    fact: String,
) -> ReviewFinding {
    ReviewFinding {
        id: String::new(),
        category: category.to_string(),
        severity,
        confidence,
        verification_status: VerificationStatus::Unverified,
        rank_tier: RankTier::Unverified,
        sources: vec![agent.to_string()],
        file: file.to_string(),
        line1: line1.max(1),
        line2: line2.max(line1).max(1),
        claim,
        evidence: vec![ReviewEvidence {
            kind: evidence_kinds::STATIC_FACT.to_string(),
            path: Some(file.to_string()),
            line1: Some(line1.max(1)),
            line2: Some(line2.max(line1).max(1)),
            content: fact,
        }],
        impact: None,
        remediation: None,
        checks_performed: vec![],
    }
}

fn map_security_severity(severity: &refact_codegraph::security_scan::Severity) -> ReviewSeverity {
    match severity {
        refact_codegraph::security_scan::Severity::Low => ReviewSeverity::Low,
        refact_codegraph::security_scan::Severity::Medium => ReviewSeverity::Medium,
        refact_codegraph::security_scan::Severity::High => ReviewSeverity::High,
        refact_codegraph::security_scan::Severity::Critical => ReviewSeverity::Critical,
    }
}

pub async fn s1_security(gcx: Arc<GlobalContext>, scope: &ReviewScope) -> AgentOutcome {
    let agent = "s1_security";
    let started = now_ms();
    let Some(service) = gcx.codegraph.lock().await.clone() else {
        return AgentOutcome::skipped(agent, "codegraph_unavailable");
    };
    let mut findings = Vec::new();
    let mut candidates = 0usize;
    for path in scope.files.iter().take(S1_MAX_FILES) {
        let path_str = path.to_string_lossy().to_string();
        let Ok(text) = get_file_text_from_memory_or_disk(gcx.clone(), path).await else {
            continue;
        };
        let lang = refact_codegraph::lang_from_path(&path_str);
        let Ok(hits) = service.security_scan(&path_str, lang, &text).await else {
            continue;
        };
        for hit in hits {
            candidates += 1;
            if findings.len() >= S1_MAX_FINDINGS {
                continue;
            }
            let snippet: String = hit.snippet.chars().take(160).collect();
            findings.push(static_finding(
                agent,
                "security",
                &path_str,
                hit.line as u32,
                hit.line as u32,
                map_security_severity(&hit.severity),
                0.85,
                format!("Security rule `{}` matches this line.", hit.rule),
                snippet,
            ));
        }
    }
    AgentOutcome::ran(agent, None, candidates, findings, started)
}

pub async fn s2_dead_code(
    gcx: Arc<GlobalContext>,
    scope: &ReviewScope,
    min_confidence: f64,
) -> AgentOutcome {
    let agent = "s2_dead_code";
    let started = now_ms();
    let Some(service) = gcx.codegraph.lock().await.clone() else {
        return AgentOutcome::skipped(agent, "codegraph_unavailable");
    };
    match service.index_readiness().await {
        Ok(readiness) if readiness.cross_file_ready => {}
        Ok(_) => return AgentOutcome::skipped(agent, "index_building"),
        Err(_) => return AgentOutcome::skipped(agent, "index_unavailable"),
    }
    let Ok(dead) = service.dead_code().await else {
        return AgentOutcome::skipped(agent, "dead_code_unavailable");
    };
    let mut findings = Vec::new();
    let mut candidates = 0usize;
    for symbol in dead {
        let Some(scoped_path) = path_in_scope(&symbol.path, scope) else {
            continue;
        };
        if symbol.confidence < min_confidence {
            continue;
        }
        candidates += 1;
        if findings.len() >= S2_MAX_FINDINGS {
            continue;
        }
        findings.push(static_finding(
            agent,
            "consistency",
            &scoped_path,
            symbol.line as u32,
            symbol.line as u32,
            ReviewSeverity::Medium,
            symbol.confidence.min(0.95) as f32,
            format!(
                "Symbol `{}` appears unreachable: {} ({} incoming edges).",
                symbol.name, symbol.reason, symbol.incoming_edges
            ),
            format!(
                "dead-code candidate `{}` at {}:{} confidence {:.2}",
                symbol.name, symbol.path, symbol.line, symbol.confidence
            ),
        ));
    }
    AgentOutcome::ran(agent, None, candidates, findings, started)
}

pub async fn s3_duplication(gcx: Arc<GlobalContext>, scope: &ReviewScope) -> AgentOutcome {
    let agent = "s3_duplication";
    let started = now_ms();
    let analysis = match crate::tools::tool_codegraph::cached_cross_file_clones(gcx.clone()).await {
        Ok(analysis) => analysis,
        Err(_) => return AgentOutcome::skipped(agent, "codegraph_unavailable"),
    };
    let mut findings = Vec::new();
    let mut candidates = 0usize;
    for clone in &analysis.clones {
        let in_a = path_in_scope(&clone.file_a, scope);
        let in_b = path_in_scope(&clone.file_b, scope);
        if in_a.is_none() && in_b.is_none() {
            continue;
        }
        candidates += 1;
        if findings.len() >= S3_MAX_FINDINGS {
            continue;
        }
        let (file, line1, line2, other, other_l1, other_l2) = match (&in_a, &in_b) {
            (Some(a), _) => (
                a.clone(),
                clone.a_start_line,
                clone.a_end_line,
                clone.file_b.clone(),
                clone.b_start_line,
                clone.b_end_line,
            ),
            (None, Some(b)) => (
                b.clone(),
                clone.b_start_line,
                clone.b_end_line,
                clone.file_a.clone(),
                clone.a_start_line,
                clone.a_end_line,
            ),
            (None, None) => unreachable!(),
        };
        let severity = if clone.token_len >= 150 {
            ReviewSeverity::Medium
        } else {
            ReviewSeverity::Low
        };
        findings.push(static_finding(
            agent,
            "maintainability",
            &file,
            line1 as u32,
            line2 as u32,
            severity,
            0.8,
            format!(
                "Lines {}-{} duplicate {}:{}-{} ({} tokens).",
                line1, line2, other, other_l1, other_l2, clone.token_len
            ),
            format!(
                "cross-file clone pair: {}:{}-{} <-> {}:{}-{}",
                file, line1, line2, other, other_l1, other_l2
            ),
        ));
    }
    AgentOutcome::ran(agent, None, candidates, findings, started)
}

fn repo_root_for(scope: &ReviewScope, gcx: &GlobalContext) -> Option<PathBuf> {
    let workspace_folders = gcx.documents_state.workspace_folders.lock().ok()?.clone();
    scope
        .files
        .iter()
        .map(|path| {
            if path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or(path.as_path())
            }
        })
        .chain(workspace_folders.iter().map(PathBuf::as_path))
        .find_map(|path| {
            let repo = refact_worktrees::git::discover_repo(path).ok()?;
            refact_worktrees::git::repo_root(&repo).ok()
        })
}

pub async fn s6_enrichment(
    gcx: Arc<GlobalContext>,
    scope: &ReviewScope,
    max_commits: usize,
) -> (RiskEnrichment, AgentRunReport) {
    let agent = "s6_git_enrichment";
    let started = now_ms();
    let mut enrichment = RiskEnrichment::default();

    let fan_in_by_path = fan_in_counts(gcx.clone()).await;

    let root = repo_root_for(scope, gcx.as_ref());
    let intel = match &root {
        Some(root) => crate::tools::tool_codegraph::cached_mine_history_async(root, max_commits)
            .await
            .ok(),
        None => None,
    };

    if intel.is_none() && fan_in_by_path.is_empty() {
        return (
            enrichment,
            AgentRunReport {
                agent: agent.to_string(),
                model: None,
                status: AgentRunStatus::Skipped,
                reason: Some("no_git_history_or_graph".to_string()),
                candidates: 0,
                survived: 0,
                duration_ms: now_ms().saturating_sub(started),
                steps: None,
            },
        );
    }

    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let temporal: Vec<String> = intel
        .as_ref()
        .map(|intel| {
            intel
                .temporal_hotspots(now_ts, 30)
                .into_iter()
                .map(|(path, _)| path)
                .collect()
        })
        .unwrap_or_default();

    let mut enriched = 0usize;
    for path in &scope.files {
        let abs = normalize(&path.to_string_lossy());
        let rel = root
            .as_ref()
            .and_then(|root| {
                Path::new(&abs)
                    .strip_prefix(root)
                    .ok()
                    .map(|p| normalize(&p.to_string_lossy()))
            })
            .unwrap_or_else(|| abs.clone());
        let mut facts = FileRiskFacts::default();
        if let Some(intel) = &intel {
            facts.churn_percentile = intel.churn_percentile(&rel);
            facts.temporal_hotspot = temporal.iter().any(|t| t == &rel);
            facts.bus_factor_risk = intel.ownership_risk(&rel);
        }
        facts.fan_in = fan_in_by_path
            .iter()
            .find_map(|(candidate, count)| {
                let c = normalize(candidate);
                (c == abs || c.ends_with(&format!("/{rel}")) || abs.ends_with(&format!("/{c}")))
                    .then_some(*count)
            })
            .unwrap_or(0);
        if facts.churn_percentile > 0.0
            || facts.temporal_hotspot
            || facts.bus_factor_risk
            || facts.fan_in > 0
        {
            enriched += 1;
        }
        enrichment.insert(&rel, facts);
    }

    (
        enrichment,
        AgentRunReport {
            agent: agent.to_string(),
            model: None,
            status: AgentRunStatus::Ran,
            reason: None,
            candidates: enriched,
            survived: 0,
            duration_ms: now_ms().saturating_sub(started),
            steps: None,
        },
    )
}

async fn fan_in_counts(gcx: Arc<GlobalContext>) -> HashMap<String, usize> {
    let Some(service) = gcx.codegraph.lock().await.clone() else {
        return HashMap::new();
    };
    let Ok(cached) = service.cached_graph_analytics().await else {
        return HashMap::new();
    };
    let mut path_by_id: HashMap<i64, &String> = HashMap::new();
    for (id, _name, path) in &cached.data.nodes {
        path_by_id.insert(*id, path);
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (src, dst, kind) in &cached.data.edges {
        if kind == "defined_in" {
            continue;
        }
        let (Some(src_path), Some(dst_path)) = (path_by_id.get(src), path_by_id.get(dst)) else {
            continue;
        };
        if src_path == dst_path {
            continue;
        }
        *counts.entry((*dst_path).clone()).or_default() += 1;
    }
    counts
}
