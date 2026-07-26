use serde::Serialize;

#[derive(Serialize)]
pub struct ScoreEntry {
    pub symbol: String,
    pub path: String,
    pub score: f64,
}

#[derive(Serialize)]
pub struct FileScoreEntry {
    pub path: String,
    pub score: f64,
}

#[derive(Serialize)]
pub struct FileCentralityResponse {
    pub top_pagerank: Vec<FileScoreEntry>,
    pub top_betweenness: Vec<FileScoreEntry>,
}

#[derive(Serialize)]
pub struct CodeIntelCounts {
    pub nodes: i64,
    pub edges: i64,
    pub files: i64,
}

#[derive(Serialize)]
pub struct OverviewResponse {
    pub counts: CodeIntelCounts,
    pub index_state: crate::tools::tool_codegraph::PrBlastIndexState,
    pub scc_count: usize,
    pub largest_scc: usize,
    pub component_count: usize,
    pub top_pagerank: Vec<ScoreEntry>,
    pub top_betweenness: Vec<ScoreEntry>,
    pub file_centrality: FileCentralityResponse,
    pub community_count: usize,
    pub dead_code_count: usize,
}

#[derive(Serialize)]
pub struct GraphNodeResponse {
    pub id: i64,
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Serialize)]
pub struct GraphEdgeResponse {
    pub source: i64,
    pub target: i64,
    pub kind: String,
}

#[derive(Serialize)]
pub struct GraphResponse {
    pub index_state: crate::tools::tool_codegraph::PrBlastIndexState,
    pub nodes: Vec<GraphNodeResponse>,
    pub edges: Vec<GraphEdgeResponse>,
}

#[derive(Serialize)]
pub struct CommunityResponse {
    pub id: usize,
    pub label: String,
    pub member_count: usize,
    pub cohesion: f64,
    pub index_state: crate::tools::tool_codegraph::PrBlastIndexState,
}

#[derive(Serialize)]
pub struct PrBlastResponse {
    pub changed_files: Vec<String>,
    pub directly_impacted: Vec<refact_codegraph::pr_blast::BlastImpact>,
    pub transitively_impacted: Vec<refact_codegraph::pr_blast::BlastImpact>,
    pub impacted_file_count: usize,
    pub risk_score: f64,
    pub suggested_reviewers: Vec<crate::tools::tool_codegraph::SuggestedReviewer>,
    pub index_state: crate::tools::tool_codegraph::PrBlastIndexState,
    pub partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Serialize)]
pub struct HealthAggregateResponse {
    pub file_count: usize,
    pub function_count: usize,
    pub avg_score: f64,
    pub grade: char,
    pub max_complexity: u32,
    pub avg_maintainability: f64,
    pub avg_maintainability_index: f64,
    pub avg_maintainability_signal: f64,
    pub avg_duplication_pct: f64,
    pub biomarker_count: usize,
    pub refactoring_count: usize,
}

#[derive(Serialize)]
pub struct HealthFunctionResponse {
    pub name: String,
    pub line1: usize,
    pub complexity: u32,
    pub nesting: u32,
    pub loc: u32,
    pub maintainability: f64,
    pub maintainability_index: f64,
}

#[derive(Serialize)]
pub struct HealthFileResponse {
    pub path: String,
    pub lang: String,
    pub score: f64,
    pub grade: char,
    pub complexity: u32,
    pub maintainability: f64,
    pub maintainability_index: f64,
    pub maintainability_signal: f64,
    pub max_complexity: u32,
    pub avg_maintainability: f64,
    pub function_count: usize,
    pub duplication_pct: f64,
    pub dry_violation: bool,
    pub defect_score: f64,
    pub maintainability_score: f64,
    pub performance_score: f64,
    pub biomarker_count: usize,
    pub refactoring_count: usize,
    pub functions: Vec<HealthFunctionResponse>,
    pub findings: Vec<crate::tools::tool_codegraph::HealthFinding>,
    pub health_impact: Vec<crate::tools::tool_codegraph::HealthImpactContributor>,
    pub cache_hit: bool,
    pub refactorings: Vec<refact_codehealth::refactoring::RefactoringSuggestion>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub index_state: crate::tools::tool_codegraph::PrBlastIndexState,
    pub aggregate: HealthAggregateResponse,
    pub files: Vec<HealthFileResponse>,
}

#[derive(Serialize)]
pub struct GitHotspotResponse {
    pub path: String,
    pub churn: u32,
    pub risk: f64,
    pub churn_risk: f64,
    pub churn_percentile: f64,
    pub temporal_score: f64,
    pub change_entropy: f64,
    pub change_entropy_pct: f64,
    pub bus_factor: usize,
    pub ownership_risk: bool,
    pub knowledge_loss: bool,
}

#[derive(Serialize)]
pub struct GitOwnerResponse {
    pub author: String,
    pub commits: u32,
    pub share: f64,
}

#[derive(Serialize)]
pub struct GitOwnershipResponse {
    pub path: String,
    pub top_owner: String,
    pub top_owner_share: f64,
    pub bus_factor: usize,
    pub owner_count: usize,
    pub ownership_risk: bool,
    pub knowledge_loss: bool,
    pub owners: Vec<GitOwnerResponse>,
}

#[derive(Serialize)]
pub struct GitCoChangeResponse {
    pub path_a: String,
    pub path_b: String,
    pub count: u32,
}

#[derive(Serialize)]
pub struct GitReviewerResponse {
    pub author: String,
    pub score: f64,
}

#[derive(Serialize)]
pub struct GitFindingResponse {
    pub path: String,
    pub biomarker: String,
    pub category: String,
    pub dimension: refact_codehealth::biomarkers::Dimension,
    pub severity: refact_codehealth::biomarkers::Severity,
    pub line: usize,
    pub detail: String,
}

#[derive(Serialize)]
pub struct GitRiskResponse {
    pub commits_analyzed: u32,
    pub agent_authored_pct: f64,
    pub hotspots: Vec<GitHotspotResponse>,
    pub ownership: Vec<GitOwnershipResponse>,
    pub co_change: Vec<GitCoChangeResponse>,
    pub coupling: Vec<refact_git_intel::coupling::CouplingEdge>,
    pub reviewers: Vec<GitReviewerResponse>,
    pub findings: Vec<GitFindingResponse>,
    pub recent_commit_risks: Vec<crate::tools::tool_codegraph::RecentCommitRiskSummary>,
}

#[derive(Serialize)]
pub struct DuplicationAggregateResponse {
    pub file_count: usize,
    pub clone_pair_count: usize,
    pub duplication_pct: f64,
    pub duplication_percent: f64,
}

#[derive(Serialize)]
pub struct DuplicationCloneResponse {
    pub path_a: String,
    pub path_b: String,
    pub line_a: usize,
    pub line_b: usize,
    pub a_start_line: usize,
    pub a_end_line: usize,
    pub b_start_line: usize,
    pub b_end_line: usize,
    pub lines: usize,
    pub token_len: usize,
    pub co_change: u32,
}

#[derive(Serialize)]
pub struct DuplicationFindingResponse {
    pub path: String,
    pub biomarker: String,
    pub category: String,
    pub dimension: refact_codehealth::biomarkers::Dimension,
    pub severity: refact_codehealth::biomarkers::Severity,
    pub line: usize,
    pub detail: String,
}

#[derive(Serialize)]
pub struct DuplicationResponse {
    pub aggregate: DuplicationAggregateResponse,
    pub clones: Vec<DuplicationCloneResponse>,
    pub dry_violations: Vec<DuplicationFindingResponse>,
    pub test_smells: Vec<DuplicationFindingResponse>,
}

#[derive(Serialize)]
pub struct SecurityScanResponse {
    pub path: String,
    pub lang: String,
    pub finding_count: usize,
    pub counts: std::collections::BTreeMap<String, usize>,
    pub findings: Vec<refact_codegraph::security_scan::SecurityFinding>,
    pub omitted: usize,
}

#[derive(Serialize)]
pub struct OverviewCommunity { pub label: String, pub member_count: usize, pub cohesion: f64 }

#[derive(Serialize)]
pub struct OverviewExecutionFlow { pub entry: String, pub reaches: usize, pub depth: usize }

#[derive(Serialize)]
pub struct OverviewDeadSymbol { pub name: String, pub path: String, pub reason: String, pub confidence: f64 }

#[derive(Serialize)]
pub struct CallGraphEdge { pub caller: String, pub callee: String }

#[derive(Serialize)]
pub struct CoverageSummary { pub label: String, pub line_pct: f64, pub branch_pct: f64, pub files_below_50: usize }

#[derive(Serialize)]
pub struct CodeMapPageLink { pub target_path: String, pub labels: Vec<String>, pub count: usize }

#[derive(Serialize)]
pub struct CodeMapPage {
    pub title: String,
    pub kind: String,
    pub score: f64,
    pub paths: Vec<String>,
    pub signals: Vec<String>,
    pub symbols: std::collections::BTreeMap<String, usize>,
    pub visibility: std::collections::BTreeMap<String, usize>,
    pub links: Vec<CodeMapPageLink>,
    pub content: String,
}

#[derive(Serialize)]
pub struct CodeMapFileScore { pub path: String, pub score: f64 }

#[derive(Serialize)]
pub struct CodeMapBacklinkHub { pub path: String, pub count: usize }

#[derive(Serialize)]
pub struct CodeWhyDecisionEntry {
    pub kind: String,
    pub confidence: f64,
    pub corroboration: usize,
    pub source_kind: String,
    pub source_ref: String,
    pub summary: String,
    pub provenance_tags: Vec<String>,
}

#[derive(Serialize)]
pub struct CodeWhyRelation { pub from: String, pub relation: String, pub to: String }

#[derive(Serialize)]
pub struct ToolJson<T: Serialize> {
    pub tool: &'static str,
    pub summary: String,
    #[serde(flatten)]
    pub data: T,
}

impl<T: Serialize> ToolJson<T> {
    pub fn new(tool: &'static str, summary: impl Into<String>, data: T) -> Self {
        Self { tool, summary: summary.into(), data }
    }
    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {e}\"}}"))
    }
}
