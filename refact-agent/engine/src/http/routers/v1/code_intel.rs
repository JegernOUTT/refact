use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::codegraph::code_intel_api::*;
use crate::custom_error::ScratchError;

const CODEGRAPH_OFF: &str = "codegraph turned off";
const OVERVIEW_TOP_N: usize = 15;
const FILE_CENTRALITY_TOP_N: usize = 100;
const DEFAULT_GRAPH_LIMIT: usize = 300;
const MAX_GRAPH_LIMIT: usize = 1000;
const DEFAULT_PR_BLAST_DEPTH: usize = 3;
const MAX_PR_BLAST_DEPTH: usize = 10;
const DEFAULT_HEALTH_LIMIT: usize = 25;
const MAX_HEALTH_LIMIT: usize = 100;
const DEFAULT_GIT_RISK_LIMIT: usize = 15;
const MAX_GIT_RISK_LIMIT: usize = 100;
const DEFAULT_DUPLICATION_LIMIT: usize = 25;
const MAX_DUPLICATION_LIMIT: usize = 100;
const MAX_FUNCTIONS_PER_FILE: usize = 30;
const MAX_FINDINGS_PER_FILE: usize = 15;
const MAX_REFACTORINGS_PER_FILE: usize = 8;
const GIT_HISTORY_MAX_COMMITS: usize = 1000;

#[derive(Serialize)]
struct DetailResponse<'a> {
    detail: &'a str,
}

#[derive(Deserialize)]
pub struct GraphQuery {
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct DeadCodeQuery {
    path: Option<String>,
    limit: Option<usize>,
    min_confidence: Option<f64>,
}

#[derive(Deserialize)]
pub struct PrBlastRequest {
    changed_files: Vec<String>,
    max_depth: Option<usize>,
}

#[derive(Deserialize)]
pub struct SecurityScanRequest {
    path: Option<String>,
    file_path: Option<String>,
    lang: Option<String>,
    text: Option<String>,
}

#[derive(Deserialize)]
pub struct CodeIntelListQuery {
    path: Option<String>,
    limit: Option<usize>,
}

fn json_response<T: Serialize>(value: &T) -> Result<Response<Body>, ScratchError> {
    let json = serde_json::to_string(value).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization problem: {e}"),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json))
        .unwrap())
}

fn detail_response(detail: &str) -> Result<Response<Body>, ScratchError> {
    json_response(&DetailResponse {
        detail,
    })
}

fn codegraph_off_response() -> Result<Response<Body>, ScratchError> {
    detail_response(CODEGRAPH_OFF)
}

fn store_error(err: String) -> ScratchError {
    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, err)
}

fn bad_request(err: impl Into<String>) -> ScratchError {
    ScratchError::new(StatusCode::BAD_REQUEST, err.into())
}

async fn codegraph_service(app: &AppState) -> Option<Arc<refact_codegraph::CodeGraphService>> {
    app.gcx.codegraph.lock().await.clone()
}

fn score_entries(entries: Vec<refact_codegraph::analytics::SymbolScore>) -> Vec<ScoreEntry> {
    entries
        .into_iter()
        .map(|entry| ScoreEntry {
            symbol: entry.symbol,
            path: entry.path,
            score: entry.score,
        })
        .collect()
}

async fn codegraph_index_state(
    service: &refact_codegraph::CodeGraphService,
) -> Result<crate::tools::tool_codegraph::PrBlastIndexState, ScratchError> {
    let readiness = service.index_readiness().await.map_err(store_error)?;
    Ok(crate::tools::tool_codegraph::pr_blast_index_state(
        &readiness,
    ))
}

fn file_score_entries(entries: Vec<(String, f64)>) -> Vec<FileScoreEntry> {
    entries
        .into_iter()
        .map(|(path, score)| FileScoreEntry {
            path,
            score,
        })
        .collect()
}

fn usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn clamped_limit(limit: Option<usize>, default: usize, max: usize) -> usize {
    limit.unwrap_or(default).clamp(1, max)
}

fn path_filter(path: Option<String>) -> Option<String> {
    path.map(|path| path.trim().replace('\\', "/"))
        .filter(|path| !path.is_empty())
}

fn path_matches_filter(path: &str, filter: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if normalized == filter || normalized.ends_with(filter) {
        return true;
    }
    let basename = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    basename == filter
}

fn health_file_response(
    analysis: crate::tools::tool_codegraph::HealthFileAnalysis,
) -> HealthFileResponse {
    let function_count = analysis.function_count;
    let functions = analysis
        .functions
        .into_iter()
        .take(MAX_FUNCTIONS_PER_FILE)
        .map(|function| HealthFunctionResponse {
            name: function.name,
            line1: function.line1,
            complexity: function.complexity,
            nesting: function.nesting,
            loc: function.loc,
            maintainability: function.maintainability,
            maintainability_index: function.maintainability_index,
        })
        .collect();
    let biomarker_count = analysis.biomarker_count;
    let findings = analysis
        .findings
        .into_iter()
        .take(MAX_FINDINGS_PER_FILE)
        .collect();
    let refactoring_count = analysis.refactoring_count;
    let mut refactorings = analysis.refactorings;
    refactorings.truncate(MAX_REFACTORINGS_PER_FILE);

    HealthFileResponse {
        path: analysis.path,
        lang: analysis.lang,
        score: analysis.defect_score,
        grade: analysis.grade,
        complexity: analysis.max_complexity,
        maintainability: analysis.maintainability_index,
        maintainability_index: analysis.maintainability_index,
        maintainability_signal: analysis.maintainability_score,
        max_complexity: analysis.max_complexity,
        avg_maintainability: analysis.avg_maintainability,
        function_count,
        duplication_pct: analysis.duplication_pct,
        dry_violation: analysis.dry_violation,
        defect_score: analysis.defect_score,
        maintainability_score: analysis.maintainability_score,
        performance_score: analysis.performance_score,
        biomarker_count,
        refactoring_count,
        functions,
        findings,
        health_impact: analysis.health_impact,
        cache_hit: analysis.cache_hit,
        refactorings,
    }
}

fn health_aggregate(files: &[HealthFileResponse]) -> HealthAggregateResponse {
    let file_count = files.len();
    let function_count = files.iter().map(|file| file.function_count).sum();
    let avg_score = if files.is_empty() {
        10.0
    } else {
        files.iter().map(|file| file.score).sum::<f64>() / files.len() as f64
    };
    let avg_maintainability = if files.is_empty() {
        100.0
    } else {
        files
            .iter()
            .map(|file| file.maintainability_index)
            .sum::<f64>()
            / files.len() as f64
    };
    let avg_maintainability_signal = if files.is_empty() {
        10.0
    } else {
        files
            .iter()
            .map(|file| file.maintainability_signal)
            .sum::<f64>()
            / files.len() as f64
    };
    let avg_duplication_pct = if files.is_empty() {
        0.0
    } else {
        files.iter().map(|file| file.duplication_pct).sum::<f64>() / files.len() as f64
    };

    HealthAggregateResponse {
        file_count,
        function_count,
        avg_score,
        grade: refact_codehealth::scoring::grade(avg_score),
        max_complexity: files
            .iter()
            .map(|file| file.max_complexity)
            .max()
            .unwrap_or(0),
        avg_maintainability,
        avg_maintainability_index: avg_maintainability,
        avg_maintainability_signal,
        avg_duplication_pct,
        biomarker_count: files.iter().map(|file| file.biomarker_count).sum(),
        refactoring_count: files.iter().map(|file| file.refactoring_count).sum(),
    }
}

fn co_change_count(
    intel: Option<&refact_git_intel::GitIntel>,
    a: &str,
    b: &str,
    project_root: Option<&Path>,
) -> u32 {
    crate::tools::tool_codegraph::co_change_count(intel, a, b, project_root)
}

fn duplication_finding_response(
    path: &str,
    finding: refact_codehealth::biomarkers::Finding,
) -> DuplicationFindingResponse {
    DuplicationFindingResponse {
        path: path.to_string(),
        biomarker: finding.biomarker,
        category: finding.category,
        dimension: finding.dimension,
        severity: finding.severity,
        line: finding.line,
        detail: finding.detail,
    }
}

fn duplication_pct_for_scope(
    analysis: &crate::tools::tool_codegraph::CloneAnalysis,
    filter: Option<&str>,
) -> f64 {
    let Some(filter) = filter else {
        return analysis.duplication_pct;
    };
    let matched_paths = analysis
        .tokens_by_path
        .keys()
        .filter(|path| path_matches_filter(path, filter))
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let total_tokens = matched_paths
        .iter()
        .map(|path| analysis.tokens_by_path.get(*path).copied().unwrap_or(0))
        .sum::<usize>();
    if total_tokens == 0 {
        return 0.0;
    }
    let duplicated_tokens = analysis
        .duplicated_tokens_by_path
        .iter()
        .filter(|(path, _)| matched_paths.contains(path.as_str()))
        .map(|(_, tokens)| *tokens)
        .sum::<usize>()
        .min(total_tokens);
    duplicated_tokens as f64 / total_tokens as f64
}

pub async fn handle_v1_code_intel_overview(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };

    let index_state = codegraph_index_state(&service).await?;
    let cached = service
        .cached_graph_analytics()
        .await
        .map_err(store_error)?;
    let overview = cached.analytics.overview.truncated(OVERVIEW_TOP_N);
    let file_centrality = cached
        .analytics
        .file_centrality
        .truncated(FILE_CENTRALITY_TOP_N);

    json_response(&OverviewResponse {
        counts: CodeIntelCounts {
            nodes: usize_to_i64(overview.node_count),
            edges: usize_to_i64(overview.edge_count),
            files: cached.counts.files,
        },
        index_state,
        scc_count: overview.scc_count,
        largest_scc: overview.largest_scc,
        component_count: overview.component_count,
        top_pagerank: score_entries(overview.top_pagerank),
        top_betweenness: score_entries(overview.top_betweenness),
        file_centrality: FileCentralityResponse {
            top_pagerank: file_score_entries(file_centrality.top_pagerank),
            top_betweenness: file_score_entries(file_centrality.top_betweenness),
        },
        community_count: cached.communities.len(),
        dead_code_count: cached.dead_code.len(),
    })
}

pub async fn handle_v1_code_intel_graph(
    State(app): State<AppState>,
    Query(query): Query<GraphQuery>,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let limit = query
        .limit
        .unwrap_or(DEFAULT_GRAPH_LIMIT)
        .clamp(1, MAX_GRAPH_LIMIT);

    let index_state = codegraph_index_state(&service).await?;
    let cached = service
        .cached_graph_analytics()
        .await
        .map_err(store_error)?;
    let node_centrality = cached.analytics.node_centrality.truncated(limit);
    let selected = select_graph_node_ids(&cached.data.nodes, &node_centrality, limit);

    let mut response_nodes = cached
        .data
        .nodes
        .iter()
        .filter(|(id, _, _)| selected.contains(id))
        .map(|(id, name, path)| GraphNodeResponse {
            id: *id,
            name: name.clone(),
            path: path.clone(),
            kind: None,
        })
        .collect::<Vec<_>>();
    response_nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut response_edges = cached
        .data
        .edges
        .iter()
        .filter(|(source, target, _)| selected.contains(source) && selected.contains(target))
        .map(|(source, target, kind)| GraphEdgeResponse {
            source: *source,
            target: *target,
            kind: kind.clone(),
        })
        .collect::<Vec<_>>();
    response_edges.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.kind.cmp(&b.kind))
    });

    json_response(&GraphResponse {
        index_state,
        nodes: response_nodes,
        edges: response_edges,
    })
}

fn select_graph_node_ids(
    nodes: &[refact_codegraph::analytics::GraphNode],
    node_centrality: &refact_codegraph::analytics::NodeCentrality,
    limit: usize,
) -> BTreeSet<i64> {
    let mut rank_by_id = std::collections::HashMap::new();
    for (index, (id, _)) in node_centrality.top_pagerank.iter().enumerate() {
        rank_by_id.entry(*id).or_insert(index);
    }
    let offset = node_centrality.top_pagerank.len();
    for (index, (id, _)) in node_centrality.top_betweenness.iter().enumerate() {
        rank_by_id.entry(*id).or_insert(offset + index);
    }

    let mut ranked = nodes
        .iter()
        .map(|(id, name, path)| {
            (
                rank_by_id.get(id).copied().unwrap_or(usize::MAX),
                *id,
                name.as_str(),
                path.as_str(),
            )
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.1.cmp(&b.1))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, id, _, _)| id)
        .collect()
}

pub async fn handle_v1_code_intel_communities(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let index_state = codegraph_index_state(&service).await?;
    let cached = service
        .cached_graph_analytics()
        .await
        .map_err(store_error)?;
    let mut communities = cached.communities.iter().collect::<Vec<_>>();
    communities.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then_with(|| a.id.cmp(&b.id))
    });
    let response = communities
        .into_iter()
        .map(|community| CommunityResponse {
            id: community.id,
            label: community.label.clone(),
            member_count: community.members.len(),
            cohesion: community.cohesion,
            index_state: index_state.clone(),
        })
        .collect::<Vec<_>>();
    json_response(&response)
}

pub async fn handle_v1_code_intel_dead_code(
    State(app): State<AppState>,
    Query(query): Query<DeadCodeQuery>,
) -> Result<Response<Body>, ScratchError> {
    if codegraph_service(&app).await.is_none() {
        return codegraph_off_response();
    }
    let limit = clamped_limit(query.limit, 500, 500);
    let filter = path_filter(query.path);
    let report = crate::tools::tool_codegraph::dead_code_report(
        app.gcx.clone(),
        limit,
        filter.as_deref(),
        query.min_confidence.unwrap_or(0.0),
    )
    .await
    .map_err(store_error)?;
    json_response(&report)
}

pub async fn handle_v1_code_intel_health(
    State(app): State<AppState>,
    Query(query): Query<CodeIntelListQuery>,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let limit = clamped_limit(query.limit, DEFAULT_HEALTH_LIMIT, MAX_HEALTH_LIMIT);
    let filter = path_filter(query.path);
    let index_state = codegraph_index_state(&service).await?;
    let project_root = crate::tools::tool_codegraph::project_dir(app.gcx.clone()).await;
    let intel = match project_root.as_deref() {
        Some(dir) => {
            crate::tools::tool_codegraph::cached_mine_history_async(dir, GIT_HISTORY_MAX_COMMITS)
                .await
                .ok()
                .map(Arc::new)
        }
        None => None,
    };
    let graph = crate::tools::tool_codegraph::health_graph_snapshot(
        Some(&service),
        project_root.as_deref(),
    )
    .await
    .map(Arc::new);
    let mut snapshots = crate::tools::tool_codegraph::load_health_snapshots(&service).await;
    let mut trend_findings = if filter.is_some() {
        refact_codehealth::trends::evaluate_trends(&snapshots)
    } else {
        Vec::new()
    };
    let git_function_fact_paths =
        crate::tools::tool_codegraph::top_git_function_fact_paths(intel.as_deref());
    let files_text = Arc::new(service.all_files_with_text().await.map_err(store_error)?);
    let mut analyses = analyze_health_files_for_response(
        files_text.clone(),
        filter.clone(),
        project_root.clone(),
        intel.clone(),
        graph.clone(),
        trend_findings.clone(),
        git_function_fact_paths.clone(),
    )
    .await?;

    if analyses.is_empty() && filter.is_some() {
        return detail_response("path is not indexed in codegraph");
    }

    if filter.is_none() {
        let snapshot = crate::tools::tool_codegraph::build_health_snapshot(&analyses);
        let mut trend_history = snapshots.clone();
        crate::tools::tool_codegraph::push_health_snapshot(&mut trend_history, snapshot.clone());
        trend_findings = refact_codehealth::trends::evaluate_trends(&trend_history);
        if !trend_findings.is_empty() {
            analyses = analyze_health_files_for_response(
                files_text.clone(),
                None,
                project_root.clone(),
                intel.clone(),
                graph.clone(),
                trend_findings.clone(),
                git_function_fact_paths.clone(),
            )
            .await?;
        }
        crate::tools::tool_codegraph::push_health_snapshot(&mut snapshots, snapshot);
        crate::tools::tool_codegraph::persist_health_snapshots(&service, &snapshots)
            .await
            .map_err(store_error)?;
    }

    let mut files = analyses
        .into_iter()
        .map(health_file_response)
        .collect::<Vec<_>>();
    let aggregate = health_aggregate(&files);
    files.sort_by(|a, b| {
        a.score
            .total_cmp(&b.score)
            .then_with(|| b.max_complexity.cmp(&a.max_complexity))
            .then_with(|| b.biomarker_count.cmp(&a.biomarker_count))
            .then_with(|| a.path.cmp(&b.path))
    });
    files.truncate(limit);

    json_response(&HealthResponse {
        index_state,
        aggregate,
        files,
    })
}

async fn analyze_health_files_for_response(
    files_text: Arc<Vec<(String, String)>>,
    filter: Option<String>,
    project_root: Option<PathBuf>,
    intel: Option<Arc<refact_git_intel::GitIntel>>,
    graph: Option<Arc<crate::tools::tool_codegraph::HealthGraphSnapshot>>,
    trend_findings: Vec<refact_codehealth::biomarkers::Finding>,
    git_function_fact_paths: HashSet<String>,
) -> Result<Vec<crate::tools::tool_codegraph::HealthFileAnalysis>, ScratchError> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::tools::tool_codegraph::HealthAnalysisContext {
            repo_root: project_root.as_deref(),
            intel: intel.as_deref(),
            graph: graph.as_deref(),
            coverage: None,
            trend_findings: &trend_findings,
            git_function_fact_paths: &git_function_fact_paths,
        };
        let mut analyses = Vec::new();
        for (path, text) in files_text.iter() {
            if filter.as_deref().is_some_and(|filter| {
                !crate::tools::tool_codegraph::health_paths_refer_to_same_file(
                    path,
                    filter,
                    project_root.as_deref(),
                )
            }) {
                continue;
            }
            analyses.push(
                crate::tools::tool_codegraph::analyze_health_file_shared(
                    path.clone(),
                    text.clone(),
                    &ctx,
                )
                .map_err(store_error)?,
            );
        }
        Ok(analyses)
    })
    .await
    .map_err(|e| store_error(format!("health analysis join: {e}")))?
}
pub async fn handle_v1_code_intel_git_risk(
    State(app): State<AppState>,
    Query(query): Query<CodeIntelListQuery>,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let limit = clamped_limit(query.limit, DEFAULT_GIT_RISK_LIMIT, MAX_GIT_RISK_LIMIT);
    let filter = path_filter(query.path);
    let Some(dir) = crate::tools::tool_codegraph::project_dir(app.gcx.clone()).await else {
        return detail_response("no project directory available");
    };
    let repo_root = super::git_read::resolve_requested_root(
        app.gcx.clone(),
        dir.to_string_lossy().into_owned(),
    )
    .await
    .ok();
    let (intel, assembly) = match repo_root {
        Some(repo_root) => {
            let intel = crate::tools::tool_codegraph::cached_mine_history_async(
                &repo_root,
                GIT_HISTORY_MAX_COMMITS,
            )
            .await
            .map_err(store_error)?;
            let assembly = crate::tools::tool_codegraph::build_git_risk_assembly(
                &intel,
                &repo_root,
                None,
                Some(&service),
                limit,
                filter.as_deref(),
            )
            .await;
            (intel, assembly)
        }
        None => (
            refact_git_intel::GitIntel::default(),
            crate::tools::tool_codegraph::GitRiskAssembly {
                files: Vec::new(),
                recent_commit_risks: Vec::new(),
            },
        ),
    };

    let hotspots = assembly
        .files
        .iter()
        .map(|file| GitHotspotResponse {
            path: file.meta.file_path.clone(),
            churn: file.meta.commit_count_total,
            risk: file.churn_risk,
            churn_risk: file.churn_risk,
            churn_percentile: file.meta.churn_percentile,
            temporal_score: file.temporal_score,
            change_entropy: file.meta.change_entropy,
            change_entropy_pct: file.meta.change_entropy_pct,
            bus_factor: file.meta.bus_factor as usize,
            ownership_risk: intel.ownership_risk(&file.meta.file_path),
            knowledge_loss: intel.knowledge_loss(&file.meta.file_path),
        })
        .collect::<Vec<_>>();

    let ownership_paths = assembly
        .files
        .iter()
        .map(|file| file.meta.file_path.clone())
        .collect::<Vec<_>>();
    let ownership = ownership_paths
        .iter()
        .map(|path| {
            let owners = intel.ownership(path);
            let top = owners.first();
            GitOwnershipResponse {
                path: path.clone(),
                top_owner: top.map(|owner| owner.author.clone()).unwrap_or_default(),
                top_owner_share: top.map(|owner| owner.share).unwrap_or(0.0),
                bus_factor: intel.bus_factor(path),
                owner_count: owners.len(),
                ownership_risk: intel.ownership_risk(path),
                knowledge_loss: intel.knowledge_loss(path),
                owners: owners
                    .into_iter()
                    .take(5)
                    .map(|owner| GitOwnerResponse {
                        author: owner.author,
                        commits: owner.commits,
                        share: owner.share,
                    })
                    .collect(),
            }
        })
        .collect::<Vec<_>>();

    let co_change = intel
        .co_change_pairs(2)
        .into_iter()
        .filter(|((a, b), _)| {
            filter.as_deref().is_none_or(|filter| {
                path_matches_filter(a, filter) || path_matches_filter(b, filter)
            })
        })
        .take(limit)
        .map(|((path_a, path_b), count)| GitCoChangeResponse {
            path_a,
            path_b,
            count,
        })
        .collect::<Vec<_>>();
    let coupling = refact_git_intel::coupling::build_coupling_graph(&intel, limit).edges;
    let reviewers =
        refact_git_intel::coupling::reviewer_suggestions(&intel, &ownership_paths, limit.min(10))
            .into_iter()
            .map(|(author, score)| GitReviewerResponse {
                author,
                score,
            })
            .collect();
    let findings = assembly
        .files
        .iter()
        .flat_map(|file| {
            file.findings
                .iter()
                .cloned()
                .map(|finding| GitFindingResponse {
                    path: file.meta.file_path.clone(),
                    biomarker: finding.biomarker,
                    category: finding.category,
                    dimension: finding.dimension,
                    severity: finding.severity,
                    line: finding.line,
                    detail: finding.detail,
                })
        })
        .collect::<Vec<_>>();

    json_response(&GitRiskResponse {
        commits_analyzed: intel.commits_analyzed,
        agent_authored_pct: intel.agent_authored_pct(),
        history_truncation: intel.history_truncation(),
        hotspots,
        ownership,
        co_change,
        coupling,
        reviewers,
        findings,
        recent_commit_risks: assembly.recent_commit_risks,
    })
}

pub async fn handle_v1_code_intel_duplication(
    State(app): State<AppState>,
    Query(query): Query<CodeIntelListQuery>,
) -> Result<Response<Body>, ScratchError> {
    let Some(_service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let limit = clamped_limit(
        query.limit,
        DEFAULT_DUPLICATION_LIMIT,
        MAX_DUPLICATION_LIMIT,
    );
    let filter = path_filter(query.path);
    let analysis = crate::tools::tool_codegraph::cached_cross_file_clones(app.gcx.clone())
        .await
        .map_err(store_error)?;
    if analysis.files == 0 {
        return json_response(&DuplicationResponse {
            aggregate: DuplicationAggregateResponse {
                file_count: 0,
                clone_pair_count: 0,
                duplication_pct: 0.0,
                duplication_percent: 0.0,
            },
            clones: Vec::new(),
            dry_violations: Vec::new(),
            test_smells: Vec::new(),
        });
    }
    let matching_file_count = match filter.as_deref() {
        Some(filter) => analysis
            .text_by_path
            .keys()
            .filter(|path| path_matches_filter(path, filter))
            .count(),
        None => analysis.files,
    };
    if filter.is_some() && matching_file_count == 0 {
        return detail_response("path is not indexed in codegraph");
    }

    let mut clones = analysis
        .clones
        .iter()
        .cloned()
        .into_iter()
        .filter(|clone| {
            filter.as_deref().is_none_or(|filter| {
                path_matches_filter(&clone.file_a, filter)
                    || path_matches_filter(&clone.file_b, filter)
            })
        })
        .collect::<Vec<_>>();
    clones.sort_by(|a, b| {
        b.token_len
            .cmp(&a.token_len)
            .then_with(|| a.file_a.cmp(&b.file_a))
            .then_with(|| a.file_b.cmp(&b.file_b))
            .then_with(|| a.a_start_line.cmp(&b.a_start_line))
            .then_with(|| a.b_start_line.cmp(&b.b_start_line))
    });
    let clone_pair_count = clones.len();
    let duplication_pct = duplication_pct_for_scope(&analysis, filter.as_deref());
    let project_root = crate::tools::tool_codegraph::project_dir(app.gcx.clone()).await;
    let intel = match project_root.as_deref() {
        Some(dir) => {
            crate::tools::tool_codegraph::cached_mine_history_async(dir, GIT_HISTORY_MAX_COMMITS)
                .await
                .ok()
        }
        None => None,
    };

    let clone_responses = clones
        .iter()
        .take(limit)
        .map(|clone| {
            let a_lines = clone
                .a_end_line
                .saturating_sub(clone.a_start_line)
                .saturating_add(1);
            let b_lines = clone
                .b_end_line
                .saturating_sub(clone.b_start_line)
                .saturating_add(1);
            DuplicationCloneResponse {
                path_a: clone.file_a.clone(),
                path_b: clone.file_b.clone(),
                line_a: clone.line_a,
                line_b: clone.line_b,
                a_start_line: clone.a_start_line,
                a_end_line: clone.a_end_line,
                b_start_line: clone.b_start_line,
                b_end_line: clone.b_end_line,
                lines: a_lines.max(b_lines),
                token_len: clone.token_len,
                co_change: co_change_count(
                    intel.as_ref(),
                    &clone.file_a,
                    &clone.file_b,
                    project_root.as_deref(),
                ),
            }
        })
        .collect::<Vec<_>>();

    let mut per_file: HashMap<String, Vec<&refact_codehealth::duplication::CrossFileClonePair>> =
        HashMap::new();
    for clone in &clones {
        per_file
            .entry(clone.file_a.clone())
            .or_default()
            .push(clone);
        per_file
            .entry(clone.file_b.clone())
            .or_default()
            .push(clone);
    }

    let mut dry_violations = Vec::new();
    let mut test_smells = Vec::new();
    for (path, file_clones) in per_file {
        if filter
            .as_deref()
            .is_some_and(|filter| !path_matches_filter(&path, filter))
        {
            continue;
        }
        let dry_clones = file_clones
            .iter()
            .map(|clone| refact_codehealth::dry::DryClonePair {
                file_a: clone.file_a.clone(),
                a_start_line: clone.a_start_line,
                a_line_count: clone
                    .a_end_line
                    .saturating_sub(clone.a_start_line)
                    .saturating_add(1),
                file_b: clone.file_b.clone(),
                b_start_line: clone.b_start_line,
                b_line_count: clone
                    .b_end_line
                    .saturating_sub(clone.b_start_line)
                    .saturating_add(1),
                co_change_count: co_change_count(
                    intel.as_ref(),
                    &clone.file_a,
                    &clone.file_b,
                    project_root.as_deref(),
                ),
            })
            .collect::<Vec<_>>();
        let file_dup_tokens = analysis
            .duplicated_tokens_by_path
            .get(&path)
            .copied()
            .unwrap_or(0);
        let file_total = analysis
            .tokens_by_path
            .get(&path)
            .copied()
            .unwrap_or(0)
            .max(1);
        let file_dup_pct = (file_dup_tokens as f64 / file_total as f64).min(1.0);
        let dry_input = refact_codehealth::dry::DryInput {
            file_path: path.clone(),
            duplication_pct: file_dup_pct,
            clones: dry_clones,
        };
        dry_violations.extend(
            refact_codehealth::dry::dry_violation(&dry_input)
                .into_iter()
                .map(|finding| duplication_finding_response(&path, finding)),
        );
        if refact_git_intel::paths::is_test_path(&path) {
            if let Some((lang, text)) = analysis.text_by_path.get(&path) {
                let assertion_blocks = refact_codehealth::assertions::assertion_blocks(lang, text);
                let ts_clones = file_clones
                    .iter()
                    .map(|clone| refact_codehealth::test_smells::ClonePair {
                        file_a: clone.file_a.clone(),
                        a_start_line: clone.a_start_line,
                        a_end_line: clone.a_end_line,
                        file_b: clone.file_b.clone(),
                        b_start_line: clone.b_start_line,
                        b_end_line: clone.b_end_line,
                    })
                    .collect();
                let ts_input = refact_codehealth::test_smells::TestSmellInput {
                    file_path: path.clone(),
                    is_test_file: true,
                    assertion_blocks,
                    clones: ts_clones,
                };
                test_smells.extend(
                    refact_codehealth::test_smells::test_smell_biomarkers(&ts_input)
                        .into_iter()
                        .map(|finding| duplication_finding_response(&path, finding)),
                );
            }
        }
    }
    dry_violations.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
    dry_violations.truncate(limit);
    test_smells.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
    test_smells.truncate(limit);

    json_response(&DuplicationResponse {
        aggregate: DuplicationAggregateResponse {
            file_count: if filter.is_some() {
                matching_file_count
            } else {
                analysis.files
            },
            clone_pair_count,
            duplication_pct,
            duplication_percent: duplication_pct * 100.0,
        },
        clones: clone_responses,
        dry_violations,
        test_smells,
    })
}

pub async fn handle_v1_code_intel_pr_blast(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let request = serde_json::from_slice::<PrBlastRequest>(&body_bytes)
        .map_err(|e| bad_request(format!("JSON problem: {e}")))?;
    let max_depth = request
        .max_depth
        .unwrap_or(DEFAULT_PR_BLAST_DEPTH)
        .clamp(1, MAX_PR_BLAST_DEPTH);
    let readiness = service.index_readiness().await.map_err(store_error)?;
    let index_state = crate::tools::tool_codegraph::pr_blast_index_state(&readiness);
    let warning = crate::tools::tool_codegraph::pr_blast_partial_warning(&index_state);
    let report = service
        .pr_blast(&request.changed_files, max_depth)
        .await
        .map_err(store_error)?;
    let repo_dir = crate::tools::tool_codegraph::project_dir(app.gcx.clone()).await;
    let intel = match repo_dir.as_deref() {
        Some(dir) => {
            crate::tools::tool_codegraph::cached_mine_history_async(dir, GIT_HISTORY_MAX_COMMITS)
                .await
                .ok()
        }
        None => None,
    };
    let suggested_reviewers = crate::tools::tool_codegraph::pr_blast_suggested_reviewers(
        &report,
        repo_dir.as_deref(),
        intel.as_ref(),
    );
    let partial = !index_state.cross_file_ready;
    json_response(&PrBlastResponse {
        changed_files: report.changed_files,
        directly_impacted: report.directly_impacted,
        transitively_impacted: report.transitively_impacted,
        impacted_file_count: report.impacted_file_count,
        risk_score: report.risk_score,
        suggested_reviewers,
        index_state,
        partial,
        warning,
    })
}

pub async fn handle_v1_code_intel_security_scan(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let request = serde_json::from_slice::<SecurityScanRequest>(&body_bytes)
        .map_err(|e| bad_request(format!("JSON problem: {e}")))?;
    let path = request.path.or(request.file_path);
    let (scan_path, lang, text) = match (request.text, path) {
        (Some(text), path) => {
            let scan_path = path.unwrap_or_default();
            let lang = request.lang.unwrap_or_else(|| {
                if scan_path.is_empty() {
                    String::new()
                } else {
                    refact_codegraph::lang_from_path(&scan_path).to_string()
                }
            });
            (scan_path, lang, text)
        }
        (None, Some(path)) => {
            if path.trim().is_empty() {
                return Err(bad_request("`path` must not be empty"));
            }
            let lang = request
                .lang
                .unwrap_or_else(|| refact_codegraph::lang_from_path(&path).to_string());
            let text = crate::files_in_workspace::get_file_text_from_memory_or_disk(
                app.gcx.clone(),
                &PathBuf::from(&path),
            )
            .await
            .map_err(bad_request)?
            .to_string();
            (path, lang, text)
        }
        (None, None) => {
            return Err(bad_request(
                "expected either `{\"path\":...}` or `{\"lang\":...,\"text\":...}`",
            ));
        }
    };

    let findings = service
        .security_scan(&scan_path, &lang, &text)
        .await
        .map_err(store_error)?;
    json_response(&findings)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use hyper::body::to_bytes;
    use tower::ServiceExt;

    use super::*;

    async fn get_json(router: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body()).await.unwrap();
        let json = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    async fn post_json(
        router: axum::Router,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body()).await.unwrap();
        let json = serde_json::from_slice(&body).unwrap();
        (status, json)
    }
}
