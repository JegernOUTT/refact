use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
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
    nodes: Vec<GraphNodeResponse>,
    edges: Vec<GraphEdgeResponse>,
}

#[derive(Serialize)]
struct CommunityResponse {
    id: usize,
    label: String,
    member_count: usize,
    cohesion: f64,
}

#[derive(Serialize)]
struct DeadCodeResponse {
    name: String,
    path: String,
    reason: String,
    confidence: f64,
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

#[derive(Serialize)]
struct HealthAggregateResponse {
    file_count: usize,
    function_count: usize,
    avg_score: f64,
    grade: char,
    max_complexity: u32,
    avg_maintainability: f64,
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
}

#[derive(Serialize)]
struct HealthFileResponse {
    path: String,
    lang: String,
    score: f64,
    grade: char,
    complexity: u32,
    maintainability: f64,
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
    findings: Vec<refact_codehealth::biomarkers::Finding>,
    refactorings: Vec<refact_codehealth::refactoring::RefactoringSuggestion>,
}

#[derive(Serialize)]
struct HealthResponse {
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
struct GitRiskResponse {
    commits_analyzed: u32,
    agent_authored_pct: f64,
    hotspots: Vec<GitHotspotResponse>,
    ownership: Vec<GitOwnershipResponse>,
    co_change: Vec<GitCoChangeResponse>,
    coupling: Vec<refact_git_intel::coupling::CouplingEdge>,
    reviewers: Vec<GitReviewerResponse>,
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

fn score_entries(entries: Vec<(String, f64)>) -> Vec<ScoreEntry> {
    entries
        .into_iter()
        .map(|(symbol, score)| ScoreEntry { symbol, score })
        .collect()
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

fn severity_rank(severity: refact_codehealth::biomarkers::Severity) -> u8 {
    match severity {
        refact_codehealth::biomarkers::Severity::Critical => 0,
        refact_codehealth::biomarkers::Severity::High => 1,
        refact_codehealth::biomarkers::Severity::Medium => 2,
        refact_codehealth::biomarkers::Severity::Low => 3,
    }
}

fn analyze_health_file(path: String, text: String) -> HealthFileResponse {
    let lang = refact_codegraph::lang_from_path(&path).to_string();
    let health = refact_codehealth::analyze(&lang, &text);
    let duplication_pct = refact_codehealth::duplication::duplication_pct(&lang, &text);
    let dry_violation = refact_codehealth::duplication::dry_violation(&lang, &text);
    let mut findings = refact_codehealth::biomarkers::detect_biomarkers(&lang, &text);
    findings.extend(refact_codehealth::perf::detect_perf(&lang, &text));
    findings.sort_by(|a, b| {
        severity_rank(a.severity)
            .cmp(&severity_rank(b.severity))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.biomarker.cmp(&b.biomarker))
    });
    let file_score = refact_codehealth::scoring::score_file(&findings);
    let mut functions = health.functions.clone();
    functions.sort_by(|a, b| {
        b.complexity
            .cmp(&a.complexity)
            .then_with(|| b.loc.cmp(&a.loc))
            .then_with(|| a.line1.cmp(&b.line1))
            .then_with(|| a.name.cmp(&b.name))
    });
    let function_count = functions.len();
    let functions = functions
        .into_iter()
        .take(MAX_FUNCTIONS_PER_FILE)
        .map(|function| HealthFunctionResponse {
            name: function.name,
            line1: function.line1,
            complexity: function.complexity,
            nesting: function.nesting,
            loc: function.loc,
            maintainability: function.maintainability,
        })
        .collect();
    let biomarker_count = findings.len();
    let findings = findings.into_iter().take(MAX_FINDINGS_PER_FILE).collect();
    let mut refactorings = refact_codehealth::refactoring::rank(
        refact_codehealth::refactoring::suggest_refactorings(&lang, &text),
    );
    let refactoring_count = refactorings.len();
    refactorings.truncate(MAX_REFACTORINGS_PER_FILE);

    HealthFileResponse {
        path,
        lang,
        score: file_score.defect,
        grade: refact_codehealth::scoring::grade(file_score.defect),
        complexity: health.max_complexity,
        maintainability: health.avg_maintainability,
        max_complexity: health.max_complexity,
        avg_maintainability: health.avg_maintainability,
        function_count,
        duplication_pct,
        dry_violation,
        defect_score: file_score.defect,
        maintainability_score: file_score.maintainability,
        performance_score: file_score.performance,
        biomarker_count,
        refactoring_count,
        functions,
        findings,
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
        files.iter().map(|file| file.maintainability).sum::<f64>() / files.len() as f64
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
        avg_duplication_pct,
        biomarker_count: files.iter().map(|file| file.biomarker_count).sum(),
        refactoring_count: files.iter().map(|file| file.refactoring_count).sum(),
    }
}

fn co_change_count(intel: Option<&refact_git_intel::GitIntel>, a: &str, b: &str) -> u32 {
    intel
        .and_then(|intel| {
            intel
                .co_change
                .get(&(a.to_string(), b.to_string()))
                .or_else(|| intel.co_change.get(&(b.to_string(), a.to_string())))
                .copied()
        })
        .unwrap_or(0)
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

pub async fn handle_v1_code_intel_overview(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };

    let counts = service.counts().await.map_err(store_error)?;
    let overview = service
        .overview(OVERVIEW_TOP_N)
        .await
        .map_err(store_error)?;
    let file_centrality = service
        .per_file_centrality(FILE_CENTRALITY_TOP_N)
        .await
        .map_err(store_error)?;
    let communities = service.communities().await.map_err(store_error)?;
    let dead_code = service.dead_code().await.map_err(store_error)?;

    json_response(&OverviewResponse {
        counts: CodeIntelCounts {
            nodes: usize_to_i64(overview.node_count),
            edges: usize_to_i64(overview.edge_count),
            files: counts.files,
        },
        scc_count: overview.scc_count,
        largest_scc: overview.largest_scc,
        component_count: overview.component_count,
        top_pagerank: score_entries(overview.top_pagerank),
        top_betweenness: score_entries(overview.top_betweenness),
        file_centrality: FileCentralityResponse {
            top_pagerank: file_score_entries(file_centrality.top_pagerank),
            top_betweenness: file_score_entries(file_centrality.top_betweenness),
        },
        community_count: communities.len(),
        dead_code_count: dead_code.len(),
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
    let mut communities = service.communities().await.map_err(store_error)?;
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
        })
        .collect::<Vec<_>>();
    json_response(&response)
}

pub async fn handle_v1_code_intel_dead_code(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let response = service
        .dead_code()
        .await
        .map_err(store_error)?
        .into_iter()
        .map(|dead| DeadCodeResponse {
            name: dead.name,
            path: dead.path,
            reason: dead.reason,
            confidence: dead.confidence,
        })
        .collect::<Vec<_>>();
    json_response(&response)
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

    let mut files = service
        .all_files_with_text()
        .await
        .map_err(store_error)?
        .into_iter()
        .filter(|(path, _)| {
            filter
                .as_deref()
                .is_none_or(|filter| path_matches_filter(path, filter))
        })
        .map(|(path, text)| analyze_health_file(path, text))
        .collect::<Vec<_>>();

    if files.is_empty() {
        if filter.is_some() {
            return detail_response("path is not indexed in codegraph");
        }
    }

    let aggregate = health_aggregate(&files);
    files.sort_by(|a, b| {
        a.score
            .total_cmp(&b.score)
            .then_with(|| b.max_complexity.cmp(&a.max_complexity))
            .then_with(|| b.biomarker_count.cmp(&a.biomarker_count))
            .then_with(|| a.path.cmp(&b.path))
    });
    files.truncate(limit);

    json_response(&HealthResponse { aggregate, files })
}

pub async fn handle_v1_code_intel_git_risk(
    State(app): State<AppState>,
    Query(query): Query<CodeIntelListQuery>,
) -> Result<Response<Body>, ScratchError> {
    let Some(_service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let limit = clamped_limit(query.limit, DEFAULT_GIT_RISK_LIMIT, MAX_GIT_RISK_LIMIT);
    let filter = path_filter(query.path);
    let Some(dir) = crate::tools::tool_codegraph::project_dir(app.gcx.clone()).await else {
        return detail_response("no project directory available");
    };
    let intel = crate::tools::tool_codegraph::cached_mine_history(&dir, GIT_HISTORY_MAX_COMMITS)
        .map_err(store_error)?;
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let temporal_map: HashMap<String, f64> =
        intel.temporal_hotspots(now_ts, limit).into_iter().collect();
    let entropy_map = intel.change_entropy();

    let hotspots = intel
        .hotspots(limit)
        .into_iter()
        .filter(|hotspot| {
            filter
                .as_deref()
                .is_none_or(|filter| path_matches_filter(&hotspot.path, filter))
        })
        .map(|hotspot| {
            let churn_risk = intel.churn_risk(&hotspot.path);
            GitHotspotResponse {
                path: hotspot.path.clone(),
                churn: hotspot.churn,
                risk: churn_risk,
                churn_risk,
                churn_percentile: intel.churn_percentile(&hotspot.path),
                temporal_score: temporal_map.get(&hotspot.path).copied().unwrap_or(0.0),
                change_entropy: entropy_map.get(&hotspot.path).copied().unwrap_or(0.0),
                change_entropy_pct: intel.change_entropy_pct(&hotspot.path),
                bus_factor: intel.bus_factor(&hotspot.path),
                ownership_risk: intel.ownership_risk(&hotspot.path),
                knowledge_loss: intel.knowledge_loss(&hotspot.path),
            }
        })
        .collect::<Vec<_>>();

    let mut ownership_paths = hotspots
        .iter()
        .map(|hotspot| hotspot.path.clone())
        .collect::<Vec<_>>();
    if ownership_paths.is_empty() && filter.is_some() {
        ownership_paths = intel
            .file_authors
            .keys()
            .filter(|path| {
                filter
                    .as_deref()
                    .is_some_and(|filter| path_matches_filter(path, filter))
            })
            .cloned()
            .collect();
        ownership_paths.sort();
        ownership_paths.truncate(limit);
    }
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

    json_response(&GitRiskResponse {
        commits_analyzed: intel.commits_analyzed,
        agent_authored_pct: intel.agent_authored_pct(),
        hotspots,
        ownership,
        co_change,
        coupling,
        reviewers,
    })
}

pub async fn handle_v1_code_intel_duplication(
    State(app): State<AppState>,
    Query(query): Query<CodeIntelListQuery>,
) -> Result<Response<Body>, ScratchError> {
    let Some(service) = codegraph_service(&app).await else {
        return codegraph_off_response();
    };
    let limit = clamped_limit(
        query.limit,
        DEFAULT_DUPLICATION_LIMIT,
        MAX_DUPLICATION_LIMIT,
    );
    let filter = path_filter(query.path);
    let files_text = service.all_files_with_text().await.map_err(store_error)?;
    if files_text.is_empty() {
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
        Some(filter) => files_text
            .iter()
            .filter(|(path, _)| path_matches_filter(path, filter))
            .count(),
        None => files_text.len(),
    };
    if filter.is_some() && matching_file_count == 0 {
        return detail_response("path is not indexed in codegraph");
    }

    let triples = files_text
        .iter()
        .map(|(path, text)| {
            (
                path.clone(),
                refact_codegraph::lang_from_path(path).to_string(),
                text.clone(),
            )
        })
        .collect::<Vec<_>>();
    let mut clones = refact_codehealth::duplication::detect_cross_file_clones(&triples)
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
    let duplication_pct = refact_codehealth::duplication::cross_file_duplication_pct(&triples);
    let intel = crate::tools::tool_codegraph::project_dir(app.gcx.clone())
        .await
        .and_then(|dir| {
            crate::tools::tool_codegraph::cached_mine_history(&dir, GIT_HISTORY_MAX_COMMITS).ok()
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
                co_change: co_change_count(intel.as_ref(), &clone.file_a, &clone.file_b),
            }
        })
        .collect::<Vec<_>>();

    let text_by_path: HashMap<String, (String, String)> = files_text
        .into_iter()
        .map(|(path, text)| {
            let lang = refact_codegraph::lang_from_path(&path).to_string();
            (path, (lang, text))
        })
        .collect();
    let tokens_by_path: HashMap<String, usize> = text_by_path
        .iter()
        .map(|(path, (lang, text))| {
            (
                path.clone(),
                refact_codehealth::duplication::tokenize(lang, text).len(),
            )
        })
        .collect();
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
                co_change_count: co_change_count(intel.as_ref(), &clone.file_a, &clone.file_b),
            })
            .collect::<Vec<_>>();
        let file_dup_tokens = file_clones
            .iter()
            .map(|clone| clone.token_len)
            .sum::<usize>();
        let file_total = tokens_by_path.get(&path).copied().unwrap_or(0).max(1);
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
            if let Some((lang, text)) = text_by_path.get(&path) {
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
                triples.len()
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
    let report = service
        .pr_blast(&request.changed_files, max_depth)
        .await
        .map_err(store_error)?;
    json_response(&report)
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

    fn commit_file(repo: &Repository, path: &str, contents: &str, msg: &str) {
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

    async fn router_with_git_risk_codegraph() -> (axum::Router, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file(
            &repo,
            "src/a.rs",
            "pub fn alpha() -> i32 { 1 }\n",
            "introduce alpha",
        );
        commit_file(
            &repo,
            "src/b.rs",
            "pub fn beta() -> i32 { 2 }\n",
            "introduce beta",
        );
        commit_file(
            &repo,
            "src/a.rs",
            "pub fn alpha() -> i32 { 3 }\n",
            "update alpha",
        );
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let codegraph = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        codegraph
            .index_file("src/a.rs", "pub fn alpha() -> i32 { 3 }\n", "rust")
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
                entry["symbol"].as_str() == Some("helper") && entry["score"].is_number()
            }));
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
        assert!(json["edges"].is_array());
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
    }

    #[tokio::test]
    async fn code_intel_git_risk_populated_returns_shape() {
        let (router, _dir) = router_with_git_risk_codegraph().await;

        let (status, json) = get_json(router, "/v1/code-intel/git-risk?limit=2").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["commits_analyzed"], 3);
        let hotspots = json["hotspots"].as_array().unwrap();
        assert!(hotspots.iter().any(|entry| {
            entry["path"].as_str() == Some("src/a.rs")
                && entry["churn"].as_u64() == Some(2)
                && entry["risk"].is_number()
        }));
        assert!(json["ownership"].as_array().unwrap().iter().any(|entry| {
            entry["path"].as_str() == Some("src/a.rs")
                && entry["top_owner"].as_str() == Some("tester@example.com")
                && entry["bus_factor"].as_u64() == Some(1)
        }));
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
}
