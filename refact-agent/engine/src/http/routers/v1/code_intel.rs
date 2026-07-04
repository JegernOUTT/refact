use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
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

#[derive(Serialize)]
struct ScoreEntry {
    symbol: String,
    path: String,
    score: f64,
}

#[derive(Serialize)]
struct FileScoreEntry {
    path: String,
    score: f64,
}

#[derive(Serialize)]
struct FileCentralityResponse {
    top_pagerank: Vec<FileScoreEntry>,
    top_betweenness: Vec<FileScoreEntry>,
}

#[derive(Serialize)]
struct CodeIntelCounts {
    nodes: i64,
    edges: i64,
    files: i64,
}

#[derive(Serialize)]
struct OverviewResponse {
    counts: CodeIntelCounts,
    index_state: crate::tools::tool_codegraph::PrBlastIndexState,
    scc_count: usize,
    largest_scc: usize,
    component_count: usize,
    top_pagerank: Vec<ScoreEntry>,
    top_betweenness: Vec<ScoreEntry>,
    file_centrality: FileCentralityResponse,
    community_count: usize,
    dead_code_count: usize,
}

#[derive(Deserialize)]
pub struct GraphQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
struct GraphNodeResponse {
    id: i64,
    name: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
}

#[derive(Serialize)]
struct GraphEdgeResponse {
    source: i64,
    target: i64,
    kind: String,
}

#[derive(Serialize)]
struct GraphResponse {
    index_state: crate::tools::tool_codegraph::PrBlastIndexState,
    nodes: Vec<GraphNodeResponse>,
    edges: Vec<GraphEdgeResponse>,
}

#[derive(Serialize)]
struct CommunityResponse {
    id: usize,
    label: String,
    member_count: usize,
    cohesion: f64,
    index_state: crate::tools::tool_codegraph::PrBlastIndexState,
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

#[derive(Serialize)]
struct PrBlastResponse {
    changed_files: Vec<String>,
    directly_impacted: Vec<refact_codegraph::pr_blast::BlastImpact>,
    transitively_impacted: Vec<refact_codegraph::pr_blast::BlastImpact>,
    impacted_file_count: usize,
    risk_score: f64,
    suggested_reviewers: Vec<crate::tools::tool_codegraph::SuggestedReviewer>,
    index_state: crate::tools::tool_codegraph::PrBlastIndexState,
    partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
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

#[derive(Serialize)]
struct HealthAggregateResponse {
    file_count: usize,
    function_count: usize,
    avg_score: f64,
    grade: char,
    max_complexity: u32,
    avg_maintainability: f64,
    avg_maintainability_index: f64,
    avg_maintainability_signal: f64,
    avg_duplication_pct: f64,
    biomarker_count: usize,
    refactoring_count: usize,
}

#[derive(Serialize)]
struct HealthFunctionResponse {
    name: String,
    line1: usize,
    complexity: u32,
    nesting: u32,
    loc: u32,
    maintainability: f64,
    maintainability_index: f64,
}

#[derive(Serialize)]
struct HealthFileResponse {
    path: String,
    lang: String,
    score: f64,
    grade: char,
    complexity: u32,
    maintainability: f64,
    maintainability_index: f64,
    maintainability_signal: f64,
    max_complexity: u32,
    avg_maintainability: f64,
    function_count: usize,
    duplication_pct: f64,
    dry_violation: bool,
    defect_score: f64,
    maintainability_score: f64,
    performance_score: f64,
    biomarker_count: usize,
    refactoring_count: usize,
    functions: Vec<HealthFunctionResponse>,
    findings: Vec<crate::tools::tool_codegraph::HealthFinding>,
    health_impact: Vec<crate::tools::tool_codegraph::HealthImpactContributor>,
    cache_hit: bool,
    refactorings: Vec<refact_codehealth::refactoring::RefactoringSuggestion>,
}

#[derive(Serialize)]
struct HealthResponse {
    index_state: crate::tools::tool_codegraph::PrBlastIndexState,
    aggregate: HealthAggregateResponse,
    files: Vec<HealthFileResponse>,
}

#[derive(Serialize)]
struct GitHotspotResponse {
    path: String,
    churn: u32,
    risk: f64,
    churn_risk: f64,
    churn_percentile: f64,
    temporal_score: f64,
    change_entropy: f64,
    change_entropy_pct: f64,
    bus_factor: usize,
    ownership_risk: bool,
    knowledge_loss: bool,
}

#[derive(Serialize)]
struct GitOwnerResponse {
    author: String,
    commits: u32,
    share: f64,
}

#[derive(Serialize)]
struct GitOwnershipResponse {
    path: String,
    top_owner: String,
    top_owner_share: f64,
    bus_factor: usize,
    owner_count: usize,
    ownership_risk: bool,
    knowledge_loss: bool,
    owners: Vec<GitOwnerResponse>,
}

#[derive(Serialize)]
struct GitCoChangeResponse {
    path_a: String,
    path_b: String,
    count: u32,
}

#[derive(Serialize)]
struct GitReviewerResponse {
    author: String,
    score: f64,
}

#[derive(Serialize)]
struct GitFindingResponse {
    path: String,
    biomarker: String,
    category: String,
    dimension: refact_codehealth::biomarkers::Dimension,
    severity: refact_codehealth::biomarkers::Severity,
    line: usize,
    detail: String,
}

#[derive(Serialize)]
struct GitRiskResponse {
    commits_analyzed: u32,
    agent_authored_pct: f64,
    hotspots: Vec<GitHotspotResponse>,
    ownership: Vec<GitOwnershipResponse>,
    co_change: Vec<GitCoChangeResponse>,
    coupling: Vec<refact_git_intel::coupling::CouplingEdge>,
    reviewers: Vec<GitReviewerResponse>,
    findings: Vec<GitFindingResponse>,
    recent_commit_risks: Vec<crate::tools::tool_codegraph::RecentCommitRiskSummary>,
}

#[derive(Serialize)]
struct DuplicationAggregateResponse {
    file_count: usize,
    clone_pair_count: usize,
    duplication_pct: f64,
    duplication_percent: f64,
}

#[derive(Serialize)]
struct DuplicationCloneResponse {
    path_a: String,
    path_b: String,
    line_a: usize,
    line_b: usize,
    a_start_line: usize,
    a_end_line: usize,
    b_start_line: usize,
    b_end_line: usize,
    lines: usize,
    token_len: usize,
    co_change: u32,
}

#[derive(Serialize)]
struct DuplicationFindingResponse {
    path: String,
    biomarker: String,
    category: String,
    dimension: refact_codehealth::biomarkers::Dimension,
    severity: refact_codehealth::biomarkers::Severity,
    line: usize,
    detail: String,
}

#[derive(Serialize)]
struct DuplicationResponse {
    aggregate: DuplicationAggregateResponse,
    clones: Vec<DuplicationCloneResponse>,
    dry_violations: Vec<DuplicationFindingResponse>,
    test_smells: Vec<DuplicationFindingResponse>,
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
    json_response(&DetailResponse { detail })
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
        .map(|(path, score)| FileScoreEntry { path, score })
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
    let nodes = cached.data.nodes;
    let edges = cached.data.edges;
    let node_centrality = cached.analytics.node_centrality.truncated(limit);
    let selected = select_graph_node_ids(&nodes, &node_centrality, limit);

    let mut response_nodes = nodes
        .into_iter()
        .filter(|(id, _, _)| selected.contains(id))
        .map(|(id, name, path)| GraphNodeResponse {
            id,
            name,
            path,
            kind: None,
        })
        .collect::<Vec<_>>();
    response_nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut response_edges = edges
        .into_iter()
        .filter(|(source, target, _)| selected.contains(source) && selected.contains(target))
        .map(|(source, target, kind)| GraphEdgeResponse {
            source,
            target,
            kind,
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
    let mut communities = service
        .cached_graph_analytics()
        .await
        .map_err(store_error)?
        .communities;
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
            label: community.label,
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
    let intel = project_root.as_deref().and_then(|dir| {
        crate::tools::tool_codegraph::cached_mine_history(dir, GIT_HISTORY_MAX_COMMITS).ok()
    });
    let mut snapshots = crate::tools::tool_codegraph::load_health_snapshots(&service).await;
    let mut trend_findings = if filter.is_some() {
        refact_codehealth::trends::evaluate_trends(&snapshots)
    } else {
        Vec::new()
    };
    let git_function_fact_paths =
        crate::tools::tool_codegraph::top_git_function_fact_paths(intel.as_ref());
    let files_text = service.all_files_with_text().await.map_err(store_error)?;
    let mut analyses = analyze_health_files_for_response(
        &files_text,
        filter.as_deref(),
        project_root.as_deref(),
        intel.as_ref(),
        &service,
        &trend_findings,
        &git_function_fact_paths,
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
                &files_text,
                None,
                project_root.as_deref(),
                intel.as_ref(),
                &service,
                &trend_findings,
                &git_function_fact_paths,
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
    files_text: &[(String, String)],
    filter: Option<&str>,
    project_root: Option<&Path>,
    intel: Option<&refact_git_intel::GitIntel>,
    service: &Arc<refact_codegraph::CodeGraphService>,
    trend_findings: &[refact_codehealth::biomarkers::Finding],
    git_function_fact_paths: &HashSet<String>,
) -> Result<Vec<crate::tools::tool_codegraph::HealthFileAnalysis>, ScratchError> {
    let ctx = crate::tools::tool_codegraph::HealthAnalysisContext {
        repo_root: project_root,
        intel,
        service: Some(service),
        coverage: None,
        trend_findings,
        git_function_fact_paths,
    };
    let mut analyses = Vec::new();
    for (path, text) in files_text {
        if filter.is_some_and(|filter| {
            !crate::tools::tool_codegraph::health_paths_refer_to_same_file(
                path,
                filter,
                project_root,
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
            .await
            .map_err(store_error)?,
        );
    }
    Ok(analyses)
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
    let intel = crate::tools::tool_codegraph::cached_mine_history(&dir, GIT_HISTORY_MAX_COMMITS)
        .map_err(store_error)?;
    let assembly = crate::tools::tool_codegraph::build_git_risk_assembly(
        &intel,
        &dir,
        Some(&service),
        limit,
        filter.as_deref(),
    )
    .await;

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
            .map(|(author, score)| GitReviewerResponse { author, score })
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
    let intel = project_root.as_deref().and_then(|dir| {
        crate::tools::tool_codegraph::cached_mine_history(dir, GIT_HISTORY_MAX_COMMITS).ok()
    });

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
    let intel = repo_dir.as_deref().and_then(|dir| {
        crate::tools::tool_codegraph::cached_mine_history(dir, GIT_HISTORY_MAX_COMMITS).ok()
    });
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
    use git2::{Repository, Signature, Time};
    use hyper::body::to_bytes;
    use tower::ServiceExt;

    use super::*;
    use crate::http::routers::make_refact_http_server;

    fn commit_file_as(
        repo: &Repository,
        path: &str,
        contents: &str,
        msg: &str,
        name: &str,
        email: &str,
    ) {
        let workdir = repo.workdir().unwrap();
        let full_path = workdir.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full_path, contents).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let time = Time::new(1_700_000_000, 0);
        let sig = Signature::new(name, email, &time).unwrap();
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .unwrap();
    }

    async fn router_with_codegraph() -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph
            .index_file(
                "src/lib.rs",
                "fn main() { helper(); }\nfn helper() {}\nfn orphan() {}\n",
                "rust",
            )
            .await
            .unwrap();
        codegraph.connect_usages().await.unwrap();
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        make_refact_http_server(app)
    }

    async fn router_with_absolute_codegraph() -> (axum::Router, String) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let core_path = repo.join("src/core.rs").to_string_lossy().to_string();
        let caller_path = repo.join("src/caller.rs").to_string_lossy().to_string();
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph
            .index_file(&core_path, "pub fn core() {}\n", "rust")
            .await
            .unwrap();
        codegraph
            .index_file(&caller_path, "fn caller() { core(); }\n", "rust")
            .await
            .unwrap();
        codegraph.connect_usages().await.unwrap();
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        (make_refact_http_server(app), caller_path)
    }

    async fn router_with_pr_blast_codegraph() -> (axum::Router, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file_as(
            &repo,
            "src/core.rs",
            "pub fn core() {}\n",
            "introduce core",
            "Caller Owner",
            "aaa-owner@example.com",
        );
        commit_file_as(
            &repo,
            "src/caller.rs",
            "fn caller() { core(); }\n",
            "introduce caller",
            "Other Caller Owner",
            "zzz-caller@example.com",
        );
        commit_file_as(
            &repo,
            "src/caller.rs",
            "fn caller() { core(); }\n// touched\n",
            "update caller",
            "Caller Owner",
            "aaa-owner@example.com",
        );
        commit_file_as(
            &repo,
            "src/base.py",
            "class Base:\n    pass\n",
            "introduce base",
            "Base Owner",
            "zzz-base@example.com",
        );
        commit_file_as(
            &repo,
            "src/derived.py",
            "class Derived(Base):\n    pass\n",
            "introduce derived",
            "Derived Owner",
            "aaa-owner@example.com",
        );
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph
            .index_file("src/core.rs", "pub fn core() {}\n", "rust")
            .await
            .unwrap();
        codegraph
            .index_file("src/caller.rs", "fn caller() { core(); }\n", "rust")
            .await
            .unwrap();
        codegraph
            .index_file("src/base.py", "class Base:\n    pass\n", "python")
            .await
            .unwrap();
        codegraph
            .index_file(
                "src/derived.py",
                "class Derived(Base):\n    pass\n",
                "python",
            )
            .await
            .unwrap();
        codegraph.connect_usages().await.unwrap();
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        (make_refact_http_server(app), dir)
    }

    async fn router_with_partial_pr_blast_codegraph() -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph
            .index_file("src/core.rs", "pub fn core() {}\n", "rust")
            .await
            .unwrap();
        codegraph
            .index_file("src/caller.rs", "fn caller() { core(); }\n", "rust")
            .await
            .unwrap();
        codegraph.connect_usages().await.unwrap();
        codegraph.enqueue_files(&["src/pending.rs".to_string()]);
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        make_refact_http_server(app)
    }

    fn git_risk_source(a: i32, b: i32, c: i32) -> String {
        format!(
            "pub fn alpha(x: i32) -> i32 {{\n    if x > 0 {{\n        if x > 1 {{\n            for i in 0..x {{\n                if i % 2 == 0 {{\n                    return {a};\n                }}\n            }}\n            return {b};\n        }}\n    }}\n    {c}\n}}\n"
        )
    }

    async fn router_with_partial_codegraph() -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph
            .index_file(
                "src/lib.rs",
                "fn main() { helper(); }\nfn helper() {}\nfn orphan() {}\n",
                "rust",
            )
            .await
            .unwrap();
        codegraph.connect_usages().await.unwrap();
        codegraph.enqueue_files(&["src/pending.rs".to_string()]);
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        make_refact_http_server(app)
    }

    async fn router_with_filtered_duplication_codegraph() -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        let cloned = co_change_clone_source("shared_alpha", "shared");
        codegraph
            .index_file("src/a.rs", &cloned, "rust")
            .await
            .unwrap();
        codegraph
            .index_file("src/copy.rs", &cloned, "rust")
            .await
            .unwrap();
        codegraph
            .index_file(
                "src/b.rs",
                "fn independent(value: i32) -> i32 { value + 1 }\n",
                "rust",
            )
            .await
            .unwrap();
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        make_refact_http_server(app)
    }

    async fn router_with_overlapping_duplication_codegraph() -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        let a = co_change_clone_source("shared_alpha", "shared-a");
        let b = co_change_clone_source("shared_beta", "shared-b");
        let c = co_change_clone_source("shared_gamma", "shared-c");
        codegraph.index_file("src/a.rs", &a, "rust").await.unwrap();
        codegraph.index_file("src/b.rs", &b, "rust").await.unwrap();
        codegraph.index_file("src/c.rs", &c, "rust").await.unwrap();
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        make_refact_http_server(app)
    }

    async fn router_with_empty_dead_code_partial_index() -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph
            .index_file("src/lib.rs", "fn main() {}\n", "rust")
            .await
            .unwrap();
        codegraph.connect_usages().await.unwrap();
        codegraph.enqueue_files(&["src/pending.rs".to_string()]);
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        make_refact_http_server(app)
    }

    async fn router_with_git_risk_codegraph() -> (axum::Router, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file_as(
            &repo,
            "src/a.rs",
            &git_risk_source(1, 1, 1),
            "introduce alpha",
            "Alice",
            "alice@example.com",
        );
        commit_file_as(
            &repo,
            "src/b.rs",
            "pub fn beta() -> i32 { 2 }\n",
            "introduce beta",
            "Bob",
            "bob@example.com",
        );
        commit_file_as(
            &repo,
            "src/a.rs",
            &git_risk_source(2, 1, 1),
            "fix crash in alpha",
            "Bob",
            "bob@example.com",
        );
        commit_file_as(
            &repo,
            "src/a.rs",
            &git_risk_source(3, 1, 1),
            "update alpha",
            "Carol",
            "carol@example.com",
        );
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph
            .index_file("src/a.rs", &git_risk_source(3, 1, 1), "rust")
            .await
            .unwrap();
        codegraph.connect_usages().await.unwrap();
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        (make_refact_http_server(app), dir)
    }

    async fn router_without_codegraph() -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        make_refact_http_server(app)
    }

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

    async fn assert_get_off_detail(uri: &str) {
        let router = router_without_codegraph().await;

        let (status, json) = get_json(router, uri).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!({"detail": "codegraph turned off"}));
    }

    async fn assert_post_off_detail(uri: &str, body: serde_json::Value) {
        let router = router_without_codegraph().await;

        let (status, json) = post_json(router, uri, body).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!({"detail": "codegraph turned off"}));
    }

    #[tokio::test]
    async fn code_intel_overview_off_returns_detail() {
        assert_get_off_detail("/v1/code-intel/overview").await;
    }

    #[tokio::test]
    async fn code_intel_graph_off_returns_detail() {
        assert_get_off_detail("/v1/code-intel/graph").await;
    }

    #[tokio::test]
    async fn code_intel_communities_off_returns_detail() {
        assert_get_off_detail("/v1/code-intel/communities").await;
    }

    #[tokio::test]
    async fn code_intel_dead_code_off_returns_detail() {
        assert_get_off_detail("/v1/code-intel/dead-code").await;
    }

    #[tokio::test]
    async fn code_intel_health_off_returns_detail() {
        assert_get_off_detail("/v1/code-intel/health").await;
    }

    #[tokio::test]
    async fn code_intel_git_risk_off_returns_detail() {
        assert_get_off_detail("/v1/code-intel/git-risk").await;
    }

    #[tokio::test]
    async fn code_intel_duplication_off_returns_detail() {
        assert_get_off_detail("/v1/code-intel/duplication").await;
    }

    #[tokio::test]
    async fn code_intel_pr_blast_off_returns_detail() {
        assert_post_off_detail(
            "/v1/code-intel/pr-blast",
            serde_json::json!({"changed_files": ["src/lib.rs"]}),
        )
        .await;
    }

    #[tokio::test]
    async fn code_intel_security_scan_off_returns_detail() {
        assert_post_off_detail(
            "/v1/code-intel/security-scan",
            serde_json::json!({"lang": "rust", "text": "fn main() {}"}),
        )
        .await;
    }

    #[tokio::test]
    async fn code_intel_overview_populated_returns_shape() {
        let router = router_with_codegraph().await;

        let (status, json) = get_json(router, "/v1/code-intel/overview").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["counts"]["files"], 1);
        assert!(json["counts"]["nodes"].as_i64().unwrap() > 0);
        assert!(json["scc_count"].is_number());
        assert!(json["top_pagerank"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["symbol"].as_str() == Some("helper")
                    && entry["path"].as_str() == Some("src/lib.rs")
                    && entry["score"].is_number()
            }));
        assert_eq!(json["index_state"]["queued"], 0);
        assert_eq!(json["index_state"]["cross_file_ready"], true);
        assert!(json["file_centrality"]["top_pagerank"].is_array());
        assert!(json["community_count"].is_number());
        assert!(json["dead_code_count"].is_number());
    }

    #[tokio::test]
    async fn code_intel_graph_populated_returns_bounded_shape() {
        let router = router_with_codegraph().await;

        let (status, json) = get_json(router, "/v1/code-intel/graph?limit=1").await;

        assert_eq!(status, StatusCode::OK);
        let nodes = json["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0]["id"].is_i64());
        assert!(nodes[0]["name"].is_string());
        assert_eq!(nodes[0]["path"], "src/lib.rs");
        assert_eq!(json["index_state"]["queued"], 0);
        assert_eq!(json["index_state"]["cross_file_ready"], true);
        assert!(json["edges"].is_array());
    }

    #[tokio::test]
    async fn all_four_endpoints_report_index_state() {
        let router = router_with_partial_codegraph().await;

        let (overview_status, overview) = get_json(router.clone(), "/v1/code-intel/overview").await;
        let (graph_status, graph) = get_json(router.clone(), "/v1/code-intel/graph").await;
        let (communities_status, communities) =
            get_json(router.clone(), "/v1/code-intel/communities").await;
        let (dead_status, dead_code) = get_json(router, "/v1/code-intel/dead-code").await;

        assert_eq!(overview_status, StatusCode::OK);
        assert_eq!(graph_status, StatusCode::OK);
        assert_eq!(communities_status, StatusCode::OK);
        assert_eq!(dead_status, StatusCode::OK);
        assert_eq!(overview["index_state"]["queued"], 1);
        assert_eq!(overview["index_state"]["cross_file_ready"], false);
        assert_eq!(graph["index_state"]["queued"], 1);
        assert_eq!(graph["index_state"]["cross_file_ready"], false);
        let communities = communities.as_array().unwrap();
        assert!(!communities.is_empty());
        assert!(communities.iter().all(|entry| {
            entry["index_state"]["queued"] == 1 && entry["index_state"]["cross_file_ready"] == false
        }));
        assert!(!dead_code["entries"].as_array().unwrap().is_empty());
        assert_eq!(dead_code["index_state"]["queued"], 1);
        assert_eq!(dead_code["index_state"]["cross_file_ready"], false);
        assert_eq!(dead_code["partial"], true);
    }

    #[tokio::test]
    async fn code_intel_dead_code_empty_response_keeps_index_state() {
        let router = router_with_empty_dead_code_partial_index().await;

        let (status, json) = get_json(router, "/v1/code-intel/dead-code").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["entries"].as_array().unwrap().len(), 0);
        assert_eq!(json["index_state"]["queued"], 1);
        assert_eq!(json["index_state"]["cross_file_ready"], false);
        assert_eq!(json["partial"], true);
        assert!(json["warning"]
            .as_str()
            .unwrap()
            .contains("index still building"));
    }

    #[tokio::test]
    async fn code_intel_health_populated_returns_shape() {
        let router = router_with_codegraph().await;

        let (status, json) = get_json(router, "/v1/code-intel/health?limit=1").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["aggregate"]["file_count"], 1);
        assert!(json["aggregate"]["avg_score"].is_number());
        let files = json["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["path"], "src/lib.rs");
        assert!(files[0]["score"].is_number());
        assert!(files[0]["grade"].is_string());
        assert!(files[0]["functions"].is_array());
        assert!(files[0]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["hot_path"].is_boolean()));
        assert_eq!(json["index_state"]["cross_file_ready"], true);
        assert!(files[0]["health_impact"].is_array());
        assert!(files[0]["maintainability_index"].is_number());
        assert!(files[0]["maintainability_signal"].is_number());
        assert!(json["aggregate"]["avg_maintainability_index"].is_number());
    }

    #[tokio::test]
    async fn code_intel_git_risk_populated_returns_shape() {
        let (router, _dir) = router_with_git_risk_codegraph().await;

        let (status, json) = get_json(router, "/v1/code-intel/git-risk?limit=2").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["commits_analyzed"], 4);
        let hotspots = json["hotspots"].as_array().unwrap();
        assert!(hotspots.iter().any(|entry| {
            entry["path"].as_str() == Some("src/a.rs")
                && entry["churn"].as_u64() == Some(3)
                && entry["risk"]
                    .as_f64()
                    .is_some_and(|risk| (0.0..=1.0).contains(&risk))
        }));
        assert!(json["ownership"].as_array().unwrap().iter().any(|entry| {
            entry["path"].as_str() == Some("src/a.rs")
                && entry["top_owner"].as_str().is_some()
                && entry["bus_factor"].as_u64().is_some_and(|bus| bus >= 1)
        }));
        assert!(json["findings"].as_array().unwrap().iter().any(|finding| {
            finding["path"].as_str() == Some("src/a.rs")
                && finding["biomarker"].as_str() == Some("prior_defect")
        }));
        assert!(!json["recent_commit_risks"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn code_intel_duplication_populated_returns_shape() {
        let router = router_with_codegraph().await;

        let (status, json) = get_json(router, "/v1/code-intel/duplication?limit=1").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["aggregate"]["file_count"], 1);
        assert!(json["aggregate"]["duplication_pct"].is_number());
        assert!(json["clones"].is_array());
        assert!(json["dry_violations"].is_array());
        assert!(json["test_smells"].is_array());
    }

    #[tokio::test]
    async fn duplication_filtered_pct_is_scope_consistent() {
        let router = router_with_filtered_duplication_codegraph().await;

        let (unfiltered_status, unfiltered) =
            get_json(router.clone(), "/v1/code-intel/duplication?limit=5").await;
        let (filtered_status, filtered) =
            get_json(router, "/v1/code-intel/duplication?path=src/b.rs&limit=5").await;

        assert_eq!(unfiltered_status, StatusCode::OK);
        assert_eq!(filtered_status, StatusCode::OK);
        assert!(unfiltered["aggregate"]["duplication_pct"]
            .as_f64()
            .is_some_and(|pct| pct > 0.0));
        assert_eq!(filtered["aggregate"]["file_count"], 1);
        assert_eq!(filtered["aggregate"]["clone_pair_count"], 0);
        assert_eq!(filtered["aggregate"]["duplication_pct"].as_f64(), Some(0.0));
        assert_eq!(
            filtered["aggregate"]["duplication_percent"].as_f64(),
            Some(0.0)
        );
    }

    #[tokio::test]
    async fn duplication_filtered_pct_uses_union_token_counts() {
        let router = router_with_overlapping_duplication_codegraph().await;

        let (status, json) =
            get_json(router, "/v1/code-intel/duplication?path=src/a.rs&limit=10").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["aggregate"]["file_count"], 1);
        assert!(json["aggregate"]["clone_pair_count"].as_u64().unwrap() >= 2);
        let filtered_pct = json["aggregate"]["duplication_pct"].as_f64().unwrap();
        assert!(
            filtered_pct > 0.9 && filtered_pct <= 1.0,
            "filtered pct out of range: {filtered_pct}"
        );
        let filtered_percent = json["aggregate"]["duplication_percent"].as_f64().unwrap();
        assert!(
            (filtered_percent - filtered_pct * 100.0).abs() < 1e-9,
            "percent must mirror pct: {filtered_percent} vs {filtered_pct}"
        );
    }

    #[test]
    fn code_intel_graph_selection_uses_node_ids_for_duplicate_names() {
        let nodes = vec![
            (1, "render".to_string(), "src/a.rs".to_string()),
            (2, "render".to_string(), "src/b.rs".to_string()),
            (3, "helper".to_string(), "src/c.rs".to_string()),
        ];
        let centrality = refact_codegraph::analytics::NodeCentrality {
            top_pagerank: vec![(2, 10.0), (3, 9.0), (1, 1.0)],
            top_betweenness: Vec::new(),
        };

        let selected = select_graph_node_ids(&nodes, &centrality, 1);

        assert_eq!(selected, BTreeSet::from([2]));
    }

    #[tokio::test]
    async fn code_intel_pr_blast_resolves_noncanonical_path() {
        let (router, caller_path) = router_with_absolute_codegraph().await;

        let (status, json) = post_json(
            router,
            "/v1/code-intel/pr-blast",
            serde_json::json!({"changed_files": [" .\\src\\core.rs "], "max_depth": 2}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["impacted_file_count"], 1);
        assert!(json["directly_impacted"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |impact| impact["path"].as_str() == Some(caller_path.as_str())
                    && impact["symbol"].as_str() == Some("caller")
            ));
    }

    #[tokio::test]
    async fn code_intel_pr_blast_returns_reviewers_kind_and_ready_state() {
        let (router, _dir) = router_with_pr_blast_codegraph().await;

        let (status, json) = post_json(
            router,
            "/v1/code-intel/pr-blast",
            serde_json::json!({"changed_files": ["src/core.rs"], "max_depth": 2}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["partial"], false);
        assert_eq!(json["index_state"]["queued"], 0);
        assert_eq!(json["index_state"]["cross_file_ready"], true);
        assert!(json["index_state"]["cross_file_edges"].as_i64().unwrap() > 0);
        assert_eq!(
            json["suggested_reviewers"][0]["author"],
            "aaa-owner@example.com"
        );
        assert!(json["directly_impacted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|impact| impact["path"].as_str() == Some("src/caller.rs")
                && impact["symbol"].as_str() == Some("caller")
                && impact["via"].as_str() == Some("calls")
                && impact["kind"].as_str() == Some("behavioral")));
    }

    #[tokio::test]
    async fn code_intel_pr_blast_returns_structural_inherits_kind() {
        let (router, _dir) = router_with_pr_blast_codegraph().await;

        let (status, json) = post_json(
            router,
            "/v1/code-intel/pr-blast",
            serde_json::json!({"changed_files": ["src/base.py"], "max_depth": 2}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(json["directly_impacted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|impact| impact["path"].as_str() == Some("src/derived.py")
                && impact["symbol"].as_str() == Some("Derived")
                && impact["via"].as_str() == Some("inherits")
                && impact["kind"].as_str() == Some("structural")));
    }

    #[tokio::test]
    async fn code_intel_pr_blast_reports_partial_index_state() {
        let router = router_with_partial_pr_blast_codegraph().await;

        let (status, json) = post_json(
            router,
            "/v1/code-intel/pr-blast",
            serde_json::json!({"changed_files": ["src/core.rs"], "max_depth": 2}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["partial"], true);
        assert_eq!(json["index_state"]["queued"], 1);
        assert_eq!(json["index_state"]["cross_file_ready"], false);
        assert!(json["warning"]
            .as_str()
            .unwrap()
            .starts_with("⚠ index still building (1 files queued"));
    }

    fn commit_files_together(repo: &Repository, files: &[(&str, &str)], msg: &str) {
        let workdir = repo.workdir().unwrap();
        let mut index = repo.index().unwrap();
        for (path, contents) in files {
            let full_path = workdir.join(path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full_path, contents).unwrap();
            index.add_path(Path::new(path)).unwrap();
        }
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let time = Time::new(1_700_000_100, 0);
        let sig = Signature::new("Tester", "tester@example.com", &time).unwrap();
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .unwrap();
    }

    fn co_change_clone_source(name: &str, marker: &str) -> String {
        let mut src =
            format!("// {marker}\nfn {name}(input: i32) -> i32 {{\n    let mut total = input;\n");
        for _ in 0..30 {
            src.push_str("    total = total + input;\n    total = total - input;\n");
        }
        src.push_str("    total\n}\n");
        src
    }

    async fn router_with_co_change_duplication() -> (axum::Router, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let a_v1 = co_change_clone_source("alpha", "v1");
        let b_v1 = co_change_clone_source("beta", "v1");
        commit_files_together(
            &repo,
            &[("src/a.rs", &a_v1), ("src/b.rs", &b_v1)],
            "first co-change",
        );
        let a_v2 = co_change_clone_source("alpha", "v2");
        let b_v2 = co_change_clone_source("beta", "v2");
        commit_files_together(
            &repo,
            &[("src/a.rs", &a_v2), ("src/b.rs", &b_v2)],
            "second co-change",
        );
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];

        let a_abs = dir.path().join("src/a.rs").to_string_lossy().to_string();
        let b_abs = dir.path().join("src/b.rs").to_string_lossy().to_string();
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph.index_file(&a_abs, &a_v2, "rust").await.unwrap();
        codegraph.index_file(&b_abs, &b_v2, "rust").await.unwrap();
        codegraph.connect_usages().await.unwrap();
        *gcx.codegraph.lock().await = Some(codegraph);
        let app = AppState::from_gcx(gcx).await;
        (make_refact_http_server(app), dir)
    }

    #[tokio::test]
    async fn code_intel_duplication_co_change_joins_absolute_paths() {
        let (router, _dir) = router_with_co_change_duplication().await;

        let (status, json) = get_json(router, "/v1/code-intel/duplication?limit=5").await;

        assert_eq!(status, StatusCode::OK);
        let clones = json["clones"].as_array().unwrap();
        assert!(
            !clones.is_empty(),
            "expected a cross-file clone between absolute-path files: {json}"
        );
        assert_eq!(
            clones[0]["co_change"].as_u64(),
            Some(2),
            "co-change join must translate absolute store paths to repo-relative git keys: {json}"
        );
    }
}
