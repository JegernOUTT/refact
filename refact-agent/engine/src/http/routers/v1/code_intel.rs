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

fn codegraph_off_response() -> Result<Response<Body>, ScratchError> {
    json_response(&DetailResponse {
        detail: CODEGRAPH_OFF,
    })
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

    let overview = service.overview(limit).await.map_err(store_error)?;
    let nodes = service.graph_nodes().await.map_err(store_error)?;
    let edges = service.graph_edges().await.map_err(store_error)?;
    let selected = select_graph_node_ids(&nodes, &overview, limit);

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
    overview: &refact_codegraph::analytics::GraphOverview,
    limit: usize,
) -> BTreeSet<i64> {
    let mut rank_by_name = HashMap::new();
    for (index, (name, _)) in overview.top_pagerank.iter().enumerate() {
        rank_by_name.entry(name.clone()).or_insert(index);
    }
    let offset = overview.top_pagerank.len();
    for (index, (name, _)) in overview.top_betweenness.iter().enumerate() {
        rank_by_name.entry(name.clone()).or_insert(offset + index);
    }

    let mut ranked = nodes
        .iter()
        .map(|(id, name, path)| {
            (
                rank_by_name.get(name).copied().unwrap_or(usize::MAX),
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
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use hyper::body::to_bytes;
    use tower::ServiceExt;

    use super::*;
    use crate::http::routers::make_refact_http_server;

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

    #[tokio::test]
    async fn code_intel_overview_off_returns_detail() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let router = make_refact_http_server(app);

        let (status, json) = get_json(router, "/v1/code-intel/overview").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!({"detail": "codegraph turned off"}));
    }

    #[tokio::test]
    async fn code_intel_graph_off_returns_detail() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let router = make_refact_http_server(app);

        let (status, json) = get_json(router, "/v1/code-intel/graph").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!({"detail": "codegraph turned off"}));
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
}
