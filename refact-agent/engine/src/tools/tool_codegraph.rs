use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use git2::{Oid, Repository};
use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::files_correction::{
    canonicalize_normalized_path, get_project_dirs, preprocess_path_for_normalization,
    registered_worktree_path_mappings, resolve_codegraph_queue_path,
};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::scope_utils::resolve_existing_path_with_execution_scope;
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};

const GIT_CACHE_TTL: Duration = Duration::from_secs(30);
const PR_BLAST_REVIEWER_LIMIT: usize = 3;
const PR_BLAST_BOT_AUTHOR_PATTERNS: [&str; 3] = ["[bot]", "agent@", "noreply"];
const DEAD_CODE_HISTORY_COMMITS: usize = 1000;
const DEAD_CODE_DEFAULT_LIMIT: usize = 50;
const DEAD_CODE_MAX_LIMIT: usize = 500;
const GIT_META_FUNCTION_FACT_TOP_K: usize = 10;
const GIT_STABLE_AGE_DAYS: i64 = 180;
const GIT_HISTORY_MAX_COMMITS: usize = 1000;
const GIT_RISK_RECENT_COMMITS: usize = 50;
const GIT_RISK_RECENT_COMMIT_LIMIT: usize = 10;
const CODE_MAP_DEFAULT_BUDGET_TOKENS: usize = 12_000;
const CODE_MAP_MAX_PAGES_WITHOUT_QUERY: usize = 60;
const CODE_MAP_QUERY_LIMIT: usize = 12;
const CODE_MAP_LINK_LIMIT: usize = 8;
const CODE_MAP_MIN_MODULE_FILES: usize = 2;
const CODE_MAP_MIN_MODULE_SIZE: u64 = 2;
const CODE_MAP_LINK_EDGE_KINDS: [&str; 3] = ["calls", "inherits", "route_handler"];
const HEALTH_SNAPSHOT_META_KEY: &str = "health_snapshots";
const HEALTH_SNAPSHOT_LIMIT: usize = 30;

type GitCacheKey = (PathBuf, Option<Oid>, usize);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SuggestedReviewer {
    pub(crate) author: String,
    pub(crate) score: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PrBlastIndexState {
    pub(crate) queued: usize,
    pub(crate) cross_file_edges: i64,
    pub(crate) cross_file_ready: bool,
}

#[derive(Default)]
struct GitMiningCache {
    history: HashMap<GitCacheKey, (Instant, refact_git_intel::GitIntel)>,
}

pub(crate) struct CloneAnalysis {
    pub(crate) generation: u64,
    pub(crate) clones: Vec<refact_codehealth::duplication::CrossFileClonePair>,
    pub(crate) duplication_pct: f64,
    pub(crate) files: usize,
    pub(crate) text_by_path: HashMap<String, (String, String)>,
    pub(crate) tokens_by_path: HashMap<String, usize>,
    pub(crate) duplicated_tokens_by_path: HashMap<String, usize>,
}

pub(crate) struct GitRiskFileAssembly {
    pub(crate) meta: refact_codehealth::git_biomarkers::GitMeta,
    pub(crate) findings: Vec<refact_codehealth::biomarkers::Finding>,
    pub(crate) temporal_score: f64,
    pub(crate) churn_risk: f64,
}

pub(crate) struct GitRiskAssembly {
    pub(crate) files: Vec<GitRiskFileAssembly>,
    pub(crate) recent_commit_risks: Vec<RecentCommitRiskSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct HealthImpactContributor {
    pub(crate) biomarker: String,
    pub(crate) category: String,
    pub(crate) dimension: refact_codehealth::biomarkers::Dimension,
    pub(crate) severity: refact_codehealth::biomarkers::Severity,
    pub(crate) line: usize,
    pub(crate) detail: String,
    pub(crate) deduction: f64,
    pub(crate) capped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct HealthFunctionSummary {
    pub(crate) name: String,
    pub(crate) line1: usize,
    pub(crate) complexity: u32,
    pub(crate) nesting: u32,
    pub(crate) loc: u32,
    pub(crate) maintainability: f64,
    pub(crate) maintainability_index: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct HealthFileAnalysis {
    pub(crate) path: String,
    pub(crate) lang: String,
    pub(crate) grade: char,
    pub(crate) maintainability_index: f64,
    pub(crate) max_complexity: u32,
    pub(crate) avg_maintainability: f64,
    pub(crate) function_count: usize,
    pub(crate) duplication_pct: f64,
    pub(crate) dry_violation: bool,
    pub(crate) defect_score: f64,
    pub(crate) maintainability_score: f64,
    pub(crate) performance_score: f64,
    pub(crate) biomarker_count: usize,
    pub(crate) refactoring_count: usize,
    pub(crate) functions: Vec<HealthFunctionSummary>,
    pub(crate) findings: Vec<HealthFinding>,
    pub(crate) health_impact: Vec<HealthImpactContributor>,
    pub(crate) impact_summaries: Vec<String>,
    pub(crate) refactorings: Vec<refact_codehealth::refactoring::RefactoringSuggestion>,
    pub(crate) cache_hit: bool,
}

#[derive(Clone)]
pub(crate) struct HealthAnalysisContext<'a> {
    pub(crate) repo_root: Option<&'a Path>,
    pub(crate) intel: Option<&'a refact_git_intel::GitIntel>,
    pub(crate) service: Option<&'a Arc<refact_codegraph::CodeGraphService>>,
    pub(crate) coverage: Option<&'a refact_codehealth::coverage::CoverageReport>,
    pub(crate) trend_findings: &'a [refact_codehealth::biomarkers::Finding],
    pub(crate) git_function_fact_paths: &'a HashSet<String>,
}

#[derive(Debug, Clone)]
struct HealthFileCore {
    path: String,
    lang: String,
    functions: Vec<HealthFunctionSummary>,
    max_complexity: u32,
    avg_maintainability: f64,
    duplication_pct: f64,
    dry_violation: bool,
    findings: Vec<HealthFinding>,
    refactorings: Vec<refact_codehealth::refactoring::RefactoringSuggestion>,
}

#[derive(Default)]
struct HealthAnalysisCache {
    files: HashMap<String, HealthCacheEntry>,
    hits: usize,
    misses: usize,
}

#[derive(Clone)]
struct HealthCacheEntry {
    content_hash: String,
    git_generation: Option<String>,
    coverage_signature: Option<String>,
    trend_signature: String,
    core: HealthFileCore,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RecentCommitRiskSummary {
    pub(crate) sha: String,
    pub(crate) summary: String,
    pub(crate) risk: f64,
    pub(crate) top_factor_names: Vec<String>,
}

#[derive(Default)]
struct CrossFileCloneCache {
    analyses: HashMap<usize, (Weak<refact_codegraph::CodeGraphService>, Arc<CloneAnalysis>)>,
}

fn git_cache() -> &'static StdMutex<GitMiningCache> {
    static CACHE: OnceLock<StdMutex<GitMiningCache>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(GitMiningCache::default()))
}

fn clone_analysis_cache() -> &'static AMutex<CrossFileCloneCache> {
    static CACHE: OnceLock<AMutex<CrossFileCloneCache>> = OnceLock::new();
    CACHE.get_or_init(|| AMutex::new(CrossFileCloneCache::default()))
}

fn health_analysis_cache() -> &'static StdMutex<HealthAnalysisCache> {
    static CACHE: OnceLock<StdMutex<HealthAnalysisCache>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HealthAnalysisCache::default()))
}

fn tool_message(tool_call_id: &String, text: String) -> Vec<ContextEnum> {
    vec![ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(text),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
        output_filter: Some(OutputFilter::no_limits()),
        ..Default::default()
    })]
}

fn codegraph_dependency() -> Vec<String> {
    vec!["codegraph".to_string()]
}

fn string_arg(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) => Err(format!("argument `{key}` is not a string: {v:?}")),
        None => Err(format!("argument `{key}` is missing")),
    }
}

fn string_list_arg(args: &HashMap<String, Value>, key: &str) -> Result<Vec<String>, String> {
    let values = match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                item.as_str()
                    .map(|item| item.trim().to_string())
                    .ok_or_else(|| format!("argument `{key}[{index}]` is not a string: {item:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.starts_with('[') {
                serde_json::from_str::<Vec<String>>(trimmed)
                    .map_err(|err| format!("argument `{key}` is not a string array: {err}"))?
            } else {
                trimmed
                    .split(',')
                    .map(|item| item.trim().to_string())
                    .collect()
            }
        }
        Some(value) => return Err(format!("argument `{key}` is not a string array: {value:?}")),
        None => return Err(format!("argument `{key}` is missing")),
    };
    Ok(values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect())
}

fn optional_usize_arg(
    args: &HashMap<String, Value>,
    key: &str,
    default: usize,
) -> Result<usize, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(default),
        Some(Value::String(value)) => value
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("argument `{key}` must be a positive integer")),
        Some(Value::Number(value)) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| format!("argument `{key}` must be a positive integer")),
        Some(value) => Err(format!(
            "argument `{key}` must be a positive integer: {value:?}"
        )),
    }
}

fn optional_f64_arg(args: &HashMap<String, Value>, key: &str, default: f64) -> Result<f64, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(default),
        Some(Value::String(value)) => value
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("argument `{key}` must be a number")),
        Some(Value::Number(value)) => value
            .as_f64()
            .ok_or_else(|| format!("argument `{key}` must be a number")),
        Some(value) => Err(format!("argument `{key}` must be a number: {value:?}")),
    }
}

fn optional_string_arg(args: &HashMap<String, Value>, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.replace('\\', "/")))
            }
        }
        Some(value) => Err(format!("argument `{key}` must be a string: {value:?}")),
    }
}

#[derive(Debug, Clone)]
struct ResolvedCodegraphFile {
    read_path: PathBuf,
    indexed_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodegraphProjectRoots {
    git_root: PathBuf,
    indexed_root: PathBuf,
}

async fn resolve_codegraph_file(
    ccx: Arc<AMutex<AtCommandsContext>>,
    raw_path: &str,
) -> Result<ResolvedCodegraphFile, String> {
    let (gcx, top_n, execution_scope) = {
        let ccx = ccx.lock().await;
        (ccx.app.gcx.clone(), ccx.top_n, ccx.execution_scope.clone())
    };
    let read_path = if let Some(resolved) =
        resolve_existing_path_with_execution_scope(gcx.clone(), execution_scope.as_ref(), raw_path)
            .await?
    {
        if resolved.outside_absolute_path {
            return Err(format!(
                "Path '{}' is outside the active worktree; CodeGraph file analysis is limited to the current project",
                resolved.path.display()
            ));
        }
        resolved.path
    } else {
        let normalized = if PathBuf::from(raw_path).is_absolute() {
            crate::files_correction::canonical_path(raw_path)
                .to_string_lossy()
                .to_string()
        } else {
            preprocess_path_for_normalization(raw_path.to_string())
        };
        let candidates = file_repair_candidates(gcx.clone(), &normalized, top_n, false).await;
        let resolved = return_one_candidate_or_a_good_error(
            gcx.clone(),
            &normalized,
            &candidates,
            &get_project_dirs(gcx.clone()).await,
            false,
        )
        .await?;
        PathBuf::from(resolved)
    };
    if !read_path.is_file() {
        return Err(format!("Path '{}' is not a file", read_path.display()));
    }
    crate::files_in_workspace::check_file_privacy_for_send(gcx.clone(), &read_path).await?;
    let indexed_path =
        if let Some(scope) = execution_scope.as_ref().filter(|scope| scope.is_enforced()) {
            scoped_indexed_path(scope, &read_path)?
                .to_string_lossy()
                .to_string()
        } else {
            let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
            resolve_codegraph_queue_path(&read_path, &mappings).store_path
        };
    Ok(ResolvedCodegraphFile {
        read_path,
        indexed_path,
    })
}

async fn resolve_codegraph_filter_path(
    ccx: Arc<AMutex<AtCommandsContext>>,
    raw_path: &str,
) -> Result<String, String> {
    let normalized_raw = raw_path.trim().replace('\\', "/");
    let (gcx, execution_scope) = {
        let ccx = ccx.lock().await;
        (ccx.app.gcx.clone(), ccx.execution_scope.clone())
    };
    if let Some(scope) = execution_scope.as_ref().filter(|scope| scope.is_enforced()) {
        scope.ensure_active_root()?;
        if !normalized_raw.contains('/') {
            return Ok(normalized_raw);
        }
        let resolved = scope.resolve_creatable_path(Path::new(&normalized_raw))?;
        if resolved.outside_absolute_path {
            return Err(format!(
                "Path '{}' is outside the active worktree; CodeGraph analysis is limited to the current project",
                resolved.path.display()
            ));
        }
        crate::files_in_workspace::check_file_privacy_for_send(gcx, &resolved.path).await?;
        return Ok(scoped_indexed_path(scope, &resolved.path)?
            .to_string_lossy()
            .to_string());
    }

    if let Ok(resolved) = resolve_codegraph_file(ccx, &normalized_raw).await {
        return Ok(resolved.indexed_path);
    }

    let normalized = PathBuf::from(&normalized_raw);
    if normalized.is_absolute() {
        let normalized = canonicalize_normalized_path(normalized);
        let project_dirs = get_project_dirs(gcx.clone()).await;
        if !project_dirs.iter().any(|root| normalized.starts_with(root)) {
            return Err(format!(
                "Path '{}' is outside of project directories:\n{:?}",
                normalized.display(),
                project_dirs
            ));
        }
        crate::files_in_workspace::check_file_privacy_for_send(gcx, &normalized).await?;
        return Ok(normalized.to_string_lossy().to_string());
    }
    Ok(preprocess_path_for_normalization(normalized_raw))
}

fn scoped_indexed_path(
    scope: &crate::worktrees::scope::ExecutionScope,
    scoped_path: &Path,
) -> Result<PathBuf, String> {
    let relative = scoped_path
        .strip_prefix(scope.effective_root())
        .map_err(|_| {
            format!(
                "Path '{}' is outside the active worktree '{}'",
                scoped_path.display(),
                scope.effective_root().display()
            )
        })?;
    Ok(canonicalize_normalized_path(
        scope.source_workspace_root().join(relative),
    ))
}

fn scoped_codegraph_project_roots(
    execution_scope: Option<&crate::worktrees::scope::ExecutionScope>,
) -> Result<Option<CodegraphProjectRoots>, String> {
    let Some(scope) = execution_scope.filter(|scope| scope.is_enforced()) else {
        return Ok(None);
    };
    scope.ensure_active_root()?;
    Ok(Some(CodegraphProjectRoots {
        git_root: scope.effective_root().to_path_buf(),
        indexed_root: scope.source_workspace_root().to_path_buf(),
    }))
}

async fn codegraph_project_roots(
    ccx: Arc<AMutex<AtCommandsContext>>,
) -> Result<Option<CodegraphProjectRoots>, String> {
    let (gcx, execution_scope) = {
        let ccx = ccx.lock().await;
        (ccx.app.gcx.clone(), ccx.execution_scope.clone())
    };
    if let Some(roots) = scoped_codegraph_project_roots(execution_scope.as_ref())? {
        return Ok(Some(roots));
    }
    Ok(project_dir(gcx).await.map(|root| CodegraphProjectRoots {
        git_root: root.clone(),
        indexed_root: root,
    }))
}

pub(crate) async fn project_dir(gcx: Arc<crate::global_context::GlobalContext>) -> Option<PathBuf> {
    crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
}

fn git_head_oid(repo_path: &Path) -> Result<Option<Oid>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let head = repo.head();
    match head {
        Ok(head) => Ok(head.target()),
        Err(error) if refact_git_intel::is_empty_head_error(&error) => Ok(None),
        Err(error) => Err(format!("git head: {error}")),
    }
}

fn git_head_unchanged(repo_path: &Path, expected: &Option<Oid>) -> bool {
    git_head_oid(repo_path)
        .ok()
        .is_some_and(|current| &current == expected)
}

pub(crate) fn cached_mine_history(
    repo_path: &Path,
    max_commits: usize,
) -> Result<refact_git_intel::GitIntel, String> {
    cached_mine_history_with(repo_path, max_commits, |path, _head, base, max| {
        refact_git_intel::mine_history_incremental(path, base, max)
    })
}

fn cached_mine_history_with<F>(
    repo_path: &Path,
    max_commits: usize,
    mine: F,
) -> Result<refact_git_intel::GitIntel, String>
where
    F: FnOnce(
        &Path,
        Option<Oid>,
        Option<refact_git_intel::GitIntel>,
        usize,
    ) -> Result<refact_git_intel::GitIntel, String>,
{
    let head = git_head_oid(repo_path)?;
    let key = (repo_path.to_path_buf(), head.clone(), max_commits);
    let now = Instant::now();
    let base = {
        let cache = git_cache().lock().unwrap();
        if let Some(intel) = cache
            .history
            .get(&key)
            .filter(|(created, _)| now.duration_since(*created) <= GIT_CACHE_TTL)
            .map(|(_, intel)| intel.clone())
        {
            return Ok(intel);
        }
        cache
            .history
            .iter()
            .filter(|((cached_path, _, cached_max), _)| {
                cached_path == repo_path && *cached_max == max_commits
            })
            .max_by_key(|(_, (created, _))| *created)
            .map(|(_, (_, intel))| intel.clone())
    };
    let intel = mine(repo_path, head.clone(), base, max_commits)?;
    if git_head_unchanged(repo_path, &head) {
        let mut cache = git_cache().lock().unwrap();
        cache.history.insert(key, (Instant::now(), intel.clone()));
    }
    Ok(intel)
}

pub(crate) fn pr_blast_index_state(
    readiness: &refact_codegraph::IndexReadiness,
) -> PrBlastIndexState {
    PrBlastIndexState {
        queued: readiness.queued,
        cross_file_edges: readiness.cross_file_edges,
        cross_file_ready: readiness.cross_file_ready,
    }
}

pub(crate) fn pr_blast_partial_warning(state: &PrBlastIndexState) -> Option<String> {
    (!state.cross_file_ready).then(|| {
        format!(
            "⚠ index still building ({} files queued, {} cross-file edges) — impact may be under-reported",
            state.queued, state.cross_file_edges
        )
    })
}

fn symbol_score_label(entry: &refact_codegraph::analytics::SymbolScore) -> String {
    if entry.path.is_empty() {
        entry.symbol.clone()
    } else {
        format!("{} ({})", entry.symbol, entry.path)
    }
}

pub(crate) fn pr_blast_suggested_reviewers(
    report: &refact_codegraph::pr_blast::BlastReport,
    repo_root: Option<&Path>,
    intel: Option<&refact_git_intel::GitIntel>,
) -> Vec<SuggestedReviewer> {
    let (Some(repo_root), Some(intel)) = (repo_root, intel) else {
        return Vec::new();
    };
    let repo_root = repo_root.to_string_lossy();
    let mut paths = BTreeSet::new();
    for path in &report.changed_files {
        paths.insert(refact_git_intel::paths::repo_relative_or_basename(
            path, &repo_root,
        ));
    }
    for impact in report
        .directly_impacted
        .iter()
        .chain(report.transitively_impacted.iter())
    {
        paths.insert(refact_git_intel::paths::repo_relative_or_basename(
            &impact.path,
            &repo_root,
        ));
    }

    let mut scores: HashMap<String, f64> = HashMap::new();
    for path in paths {
        for owner in intel.ownership(&path) {
            if owner.share <= 0.0 || pr_blast_is_bot_author(&owner.author) {
                continue;
            }
            *scores.entry(owner.author).or_insert(0.0) += owner.share;
        }
    }

    let mut reviewers: Vec<SuggestedReviewer> = scores
        .into_iter()
        .map(|(author, score)| SuggestedReviewer { author, score })
        .collect();
    reviewers.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.author.cmp(&b.author))
    });
    reviewers.truncate(PR_BLAST_REVIEWER_LIMIT);
    reviewers
}

pub(crate) fn pr_blast_is_bot_author(author: &str) -> bool {
    let author = author.to_ascii_lowercase();
    PR_BLAST_BOT_AUTHOR_PATTERNS
        .iter()
        .any(|pattern| author.contains(pattern))
}

pub(crate) async fn cached_cross_file_clones(
    gcx: Arc<crate::global_context::GlobalContext>,
) -> Result<Arc<CloneAnalysis>, String> {
    let service = gcx
        .codegraph
        .lock()
        .await
        .clone()
        .ok_or_else(|| "codegraph is not available".to_string())?;
    let key = Arc::as_ptr(&service) as usize;
    {
        let generation = service.graph_generation();
        if let Some(analysis) = cached_clone_analysis(&service, key, generation).await {
            return Ok(analysis);
        }

        let files_text = service.all_files_with_text().await?;
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
        let clones = refact_codehealth::duplication::detect_cross_file_clones(&triples);
        let duplication_pct = refact_codehealth::duplication::cross_file_duplication_pct(&triples);
        let duplicated_tokens_by_path =
            refact_codehealth::duplication::per_file_duplicated_token_counts(&triples);
        let files = files_text.len();
        let text_by_path = files_text
            .into_iter()
            .map(|(path, text)| {
                let lang = refact_codegraph::lang_from_path(&path).to_string();
                (path, (lang, text))
            })
            .collect::<HashMap<_, _>>();
        let tokens_by_path = text_by_path
            .iter()
            .map(|(path, (lang, text))| {
                (
                    path.clone(),
                    refact_codehealth::duplication::tokenize(lang, text).len(),
                )
            })
            .collect();
        let rebuilt = Arc::new(CloneAnalysis {
            generation,
            clones,
            duplication_pct,
            files,
            text_by_path,
            tokens_by_path,
            duplicated_tokens_by_path,
        });

        let mut cache = clone_analysis_cache().lock().await;
        cache
            .analyses
            .retain(|_, (weak, _)| weak.upgrade().is_some());
        let current_generation = service.graph_generation();
        if let Some((weak, cached)) = cache.analyses.get(&key) {
            if weak
                .upgrade()
                .as_ref()
                .is_some_and(|cached_service| Arc::ptr_eq(cached_service, &service))
                && cached.generation == current_generation
            {
                return Ok(cached.clone());
            }
        }
        if current_generation == generation {
            cache
                .analyses
                .insert(key, (Arc::downgrade(&service), rebuilt.clone()));
        }
        // The graph may keep advancing during active indexing; a stale-but-fresh
        // analysis is still valid for this request. Returning it (uncached when
        // the generation moved) avoids livelocking/recomputing until quiescence.
        return Ok(rebuilt);
    }
}

async fn cached_clone_analysis(
    service: &Arc<refact_codegraph::CodeGraphService>,
    key: usize,
    generation: u64,
) -> Option<Arc<CloneAnalysis>> {
    let cache = clone_analysis_cache().lock().await;
    cache.analyses.get(&key).and_then(|(weak, analysis)| {
        weak.upgrade().and_then(|cached_service| {
            (Arc::ptr_eq(&cached_service, service) && analysis.generation == generation)
                .then(|| analysis.clone())
        })
    })
}

pub(crate) fn co_change_count(
    intel: Option<&refact_git_intel::GitIntel>,
    a: &str,
    b: &str,
    project_root: Option<&Path>,
) -> u32 {
    let Some(intel) = intel else {
        return 0;
    };
    let key_a = co_change_key(a, project_root);
    let key_b = co_change_key(b, project_root);
    intel
        .co_change
        .get(&(key_a.clone(), key_b.clone()))
        .or_else(|| intel.co_change.get(&(key_b, key_a)))
        .copied()
        .unwrap_or(0)
}

fn co_change_key(path: &str, project_root: Option<&Path>) -> String {
    match project_root {
        Some(root) => {
            refact_git_intel::paths::repo_relative_or_basename(path, &root.to_string_lossy())
        }
        None => refact_git_intel::paths::normalize_separators(path),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DeadCodeIndexState {
    pub(crate) queued: usize,
    pub(crate) dirty_paths: i64,
    pub(crate) pending_refs: i64,
    pub(crate) cross_file_edges: i64,
    pub(crate) cross_file_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DeadCodeEntry {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) line: usize,
    pub(crate) reason: String,
    pub(crate) confidence: f64,
    pub(crate) git_recency: String,
    pub(crate) incoming_edges: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DeadCodeReport {
    pub(crate) entries: Vec<DeadCodeEntry>,
    #[serde(skip)]
    pub(crate) total_candidates: usize,
    pub(crate) index_state: DeadCodeIndexState,
    pub(crate) partial: bool,
    pub(crate) warning: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DeadCodeGitFacts {
    pub(crate) last_touched_days: Option<u64>,
    pub(crate) file_churn: u32,
    pub(crate) recent_churn_days: Option<u64>,
}

pub(crate) async fn dead_code_report(
    gcx: Arc<crate::global_context::GlobalContext>,
    limit: usize,
    path_filter: Option<&str>,
    min_confidence: f64,
) -> Result<DeadCodeReport, String> {
    let repo_dir = project_dir(gcx.clone()).await;
    dead_code_report_with_roots(
        gcx,
        limit,
        path_filter,
        min_confidence,
        repo_dir.as_deref(),
        repo_dir.as_deref(),
    )
    .await
}

async fn dead_code_report_with_roots(
    gcx: Arc<crate::global_context::GlobalContext>,
    limit: usize,
    path_filter: Option<&str>,
    min_confidence: f64,
    git_root: Option<&Path>,
    indexed_root: Option<&Path>,
) -> Result<DeadCodeReport, String> {
    let service = gcx
        .codegraph
        .lock()
        .await
        .clone()
        .ok_or_else(|| "codegraph is not available".to_string())?;
    let readiness = service.index_readiness().await?;
    let index_state = dead_code_index_state(&readiness);
    let warning = dead_code_partial_warning(&index_state);
    let dead = service.dead_code().await?;
    let intel = git_root.and_then(|dir| cached_mine_history(dir, DEAD_CODE_HISTORY_COMMITS).ok());
    let now_ts = current_unix_ts();
    let facts = dead_code_git_facts_by_node(&dead, indexed_root, intel.as_ref(), now_ts);
    let min_confidence = min_confidence.clamp(0.0, 1.0);
    let limit = limit.clamp(1, DEAD_CODE_MAX_LIMIT);
    let mut entries = enrich_dead_symbols_with_facts(dead, &facts)
        .into_iter()
        .filter(|entry| entry.confidence >= min_confidence)
        .filter(|entry| {
            path_filter.is_none_or(|filter| dead_code_path_matches(&entry.path, filter))
        })
        .collect::<Vec<_>>();
    let total_candidates = entries.len();
    entries.truncate(limit);
    Ok(DeadCodeReport {
        entries,
        total_candidates,
        index_state,
        partial: !readiness.cross_file_ready,
        warning,
    })
}

pub(crate) fn dead_code_index_state(
    readiness: &refact_codegraph::IndexReadiness,
) -> DeadCodeIndexState {
    DeadCodeIndexState {
        queued: readiness.queued,
        dirty_paths: readiness.dirty_paths,
        pending_refs: readiness.pending_refs,
        cross_file_edges: readiness.cross_file_edges,
        cross_file_ready: readiness.cross_file_ready,
    }
}

pub(crate) fn dead_code_partial_warning(state: &DeadCodeIndexState) -> Option<String> {
    (!state.cross_file_ready).then(|| {
        format!(
            "⚠ index still building ({} files queued, {} dirty paths, {} pending refs) — dead-code may be over-reported",
            state.queued, state.dirty_paths, state.pending_refs
        )
    })
}

pub(crate) fn enrich_dead_symbols_with_facts(
    dead: Vec<refact_codegraph::dead_code::DeadSymbol>,
    facts_by_node: &HashMap<i64, DeadCodeGitFacts>,
) -> Vec<DeadCodeEntry> {
    let mut entries = dead
        .into_iter()
        .map(|symbol| {
            let facts = facts_by_node
                .get(&symbol.node_id)
                .cloned()
                .unwrap_or_default();
            DeadCodeEntry {
                name: symbol.name,
                path: symbol.path,
                line: symbol.line,
                reason: symbol.reason,
                confidence: dead_code_confidence(symbol.incoming_edges, &facts),
                git_recency: dead_code_git_recency(&facts),
                incoming_edges: symbol.incoming_edges,
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

fn dead_code_confidence(incoming_edges: usize, facts: &DeadCodeGitFacts) -> f64 {
    let mut confidence: f64 = 0.4;
    if facts
        .last_touched_days
        .is_some_and(|age_days| age_days > 365)
        && facts.file_churn == 0
    {
        confidence += 0.3;
    }
    if incoming_edges == 0 {
        confidence += 0.1;
    }
    if facts
        .recent_churn_days
        .is_some_and(|age_days| age_days <= 90)
    {
        confidence -= 0.2;
    }
    confidence.clamp(0.1, 0.9)
}

fn dead_code_git_recency(facts: &DeadCodeGitFacts) -> String {
    match facts.last_touched_days {
        Some(days) => format!(
            "last touched {}d ago; churn {} in mined window",
            days, facts.file_churn
        ),
        None if facts.file_churn == 0 => "no git touches in mined window".to_string(),
        None => format!("churn {} in mined window", facts.file_churn),
    }
}

fn dead_code_git_facts_by_node(
    dead: &[refact_codegraph::dead_code::DeadSymbol],
    repo_root: Option<&Path>,
    intel: Option<&refact_git_intel::GitIntel>,
    now_ts: i64,
) -> HashMap<i64, DeadCodeGitFacts> {
    let mut facts_by_node = HashMap::new();
    let mut ranges_by_path: HashMap<String, Vec<(String, usize, usize)>> = HashMap::new();
    for symbol in dead {
        let key = dead_code_git_key(&symbol.path, repo_root);
        let facts = DeadCodeGitFacts {
            last_touched_days: None,
            file_churn: intel
                .and_then(|intel| intel.file_churn.get(&key).copied())
                .unwrap_or(0),
            recent_churn_days: intel.and_then(|intel| recent_churn_days(intel, &key, now_ts)),
        };
        facts_by_node.insert(symbol.node_id, facts);
        if repo_root.is_some() {
            ranges_by_path.entry(key).or_default().push((
                symbol.node_id.to_string(),
                symbol.line.max(1),
                symbol.line.max(1),
            ));
        }
    }
    let Some(repo_root) = repo_root else {
        return facts_by_node;
    };
    for (path, ranges) in ranges_by_path {
        let Ok(range_facts) = refact_git_intel::blame::range_facts(repo_root, &path, &ranges)
        else {
            continue;
        };
        for range in range_facts {
            let Ok(node_id) = range.name.parse::<i64>() else {
                continue;
            };
            if let Some(facts) = facts_by_node.get_mut(&node_id) {
                facts.last_touched_days = days_since_ts(now_ts, range.last_modified_ts);
            }
        }
    }
    facts_by_node
}

fn dead_code_git_key(path: &str, repo_root: Option<&Path>) -> String {
    match repo_root {
        Some(root) => {
            refact_git_intel::paths::repo_relative_or_basename(path, &root.to_string_lossy())
        }
        None => refact_git_intel::paths::normalize_separators(path),
    }
}

fn recent_churn_days(
    intel: &refact_git_intel::GitIntel,
    git_key: &str,
    now_ts: i64,
) -> Option<u64> {
    intel
        .commit_records
        .iter()
        .filter(|commit| commit_touches_path(commit, git_key))
        .filter_map(|commit| days_since_ts(now_ts, commit.ts))
        .min()
}

fn commit_touches_path(commit: &refact_git_intel::CommitRecord, git_key: &str) -> bool {
    commit
        .files
        .iter()
        .any(|(path, _, _)| refact_git_intel::paths::normalize_separators(path) == git_key)
}

fn days_since_ts(now_ts: i64, ts: i64) -> Option<u64> {
    if ts <= 0 {
        return None;
    }
    Some(now_ts.saturating_sub(ts).max(0) as u64 / 86_400)
}

fn current_unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

pub(crate) fn git_churn_by_file(intel: &refact_git_intel::GitIntel) -> HashMap<String, u32> {
    intel.file_churn.clone()
}

pub(crate) fn top_git_function_fact_paths(
    intel: Option<&refact_git_intel::GitIntel>,
) -> HashSet<String> {
    intel
        .map(|intel| {
            intel
                .hotspots(GIT_META_FUNCTION_FACT_TOP_K)
                .into_iter()
                .map(|hotspot| hotspot.path)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) async fn load_health_snapshots(
    service: &refact_codegraph::CodeGraphService,
) -> Vec<refact_codehealth::trends::HealthSnapshot> {
    service
        .meta_get(HEALTH_SNAPSHOT_META_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub(crate) async fn persist_health_snapshots(
    service: &refact_codegraph::CodeGraphService,
    snapshots: &[refact_codehealth::trends::HealthSnapshot],
) -> Result<(), String> {
    let value = serde_json::to_string(snapshots)
        .map_err(|err| format!("health snapshot serialize: {err}"))?;
    service.meta_set(HEALTH_SNAPSHOT_META_KEY, &value).await
}

pub(crate) fn build_health_snapshot(
    files: &[HealthFileAnalysis],
) -> refact_codehealth::trends::HealthSnapshot {
    let per_file = files
        .iter()
        .map(|file| (file.path.clone(), file.defect_score))
        .collect::<HashMap<_, _>>();
    let aggregate = if files.is_empty() {
        10.0
    } else {
        files.iter().map(|file| file.defect_score).sum::<f64>() / files.len() as f64
    };
    refact_codehealth::trends::HealthSnapshot {
        ts: current_unix_ts(),
        per_file,
        aggregate,
    }
}

pub(crate) fn push_health_snapshot(
    history: &mut Vec<refact_codehealth::trends::HealthSnapshot>,
    snapshot: refact_codehealth::trends::HealthSnapshot,
) {
    refact_codehealth::trends::push_snapshot(history, snapshot, HEALTH_SNAPSHOT_LIMIT);
}

pub(crate) async fn analyze_health_file_shared(
    path: String,
    text: String,
    ctx: &HealthAnalysisContext<'_>,
) -> Result<HealthFileAnalysis, String> {
    let lang = refact_codegraph::lang_from_path(&path).to_string();
    let content_hash = health_content_hash(&text, &lang);
    let git_generation = health_git_generation(ctx.intel);
    let coverage_signature = health_coverage_signature(ctx.coverage, &path, ctx.repo_root);
    let trend_signature = health_trend_signature(ctx.trend_findings, &path, ctx.repo_root);
    if let Some(core) = health_cache_lookup(
        &path,
        &content_hash,
        &git_generation,
        &coverage_signature,
        &trend_signature,
    ) {
        return Ok(health_analysis_from_core(core, true));
    }

    let core = compute_health_core(path, text, lang, ctx).await?;
    health_cache_store(
        &core.path,
        content_hash,
        git_generation,
        coverage_signature,
        trend_signature,
        core.clone(),
    );
    Ok(health_analysis_from_core(core, false))
}

fn health_content_hash(text: &str, lang: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(lang.as_bytes());
    hasher.update([0xff]);
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

fn hash_string(value: impl AsRef<[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_ref());
    hex::encode(hasher.finalize())
}

fn health_git_generation(intel: Option<&refact_git_intel::GitIntel>) -> Option<String> {
    intel.map(|intel| {
        if let Some(last) = &intel.last_commit_id {
            return last.clone();
        }
        let mut parts = vec![format!("commits={}", intel.commits_analyzed)];
        let mut churn = intel.file_churn.iter().collect::<Vec<_>>();
        churn.sort_by(|a, b| a.0.cmp(b.0));
        for (path, count) in churn {
            parts.push(format!("churn:{path}:{count}"));
        }
        let mut fixes = intel.fix_commit_counts.iter().collect::<Vec<_>>();
        fixes.sort_by(|a, b| a.0.cmp(b.0));
        for (path, count) in fixes {
            parts.push(format!("fix:{path}:{count}"));
        }
        hash_string(parts.join("\n"))
    })
}

fn health_cache_lookup(
    path: &str,
    content_hash: &str,
    git_generation: &Option<String>,
    coverage_signature: &Option<String>,
    trend_signature: &str,
) -> Option<HealthFileCore> {
    let mut cache = health_analysis_cache().lock().unwrap();
    let core = cache
        .files
        .get(path)
        .filter(|entry| {
            entry.content_hash == content_hash
                && &entry.git_generation == git_generation
                && &entry.coverage_signature == coverage_signature
                && entry.trend_signature == trend_signature
        })
        .map(|entry| entry.core.clone());
    if core.is_some() {
        cache.hits += 1;
    } else {
        cache.misses += 1;
    }
    core
}

fn health_cache_store(
    path: &str,
    content_hash: String,
    git_generation: Option<String>,
    coverage_signature: Option<String>,
    trend_signature: String,
    core: HealthFileCore,
) {
    health_analysis_cache().lock().unwrap().files.insert(
        path.to_string(),
        HealthCacheEntry {
            content_hash,
            git_generation,
            coverage_signature,
            trend_signature,
            core,
        },
    );
}

async fn compute_health_core(
    path: String,
    text: String,
    lang: String,
    ctx: &HealthAnalysisContext<'_>,
) -> Result<HealthFileCore, String> {
    let health = refact_codehealth::analyze(&lang, &text);
    let duplication_pct = refact_codehealth::duplication::duplication_pct(&lang, &text);
    let mut findings = refact_codehealth::biomarkers::detect_biomarkers(&lang, &text);
    findings.extend(refact_codehealth::perf::detect_perf(&lang, &text));
    let dry_findings = health_dry_findings(&path, &lang, &text, duplication_pct);
    let dry_violation = !dry_findings.is_empty();
    findings.extend(dry_findings);
    findings.extend(health_git_findings(&path, ctx, &health).await?);
    findings.extend(health_coverage_findings(
        &path,
        ctx.coverage,
        ctx.repo_root,
        ctx.intel,
        health.max_complexity,
    ));
    findings.extend(health_trend_findings_for_path(
        ctx.trend_findings,
        &path,
        ctx.repo_root,
    ));
    sort_health_findings(&mut findings);
    let findings = enrich_health_findings(ctx.service, &path, &health.functions, findings).await;

    let mut functions = health.functions.clone();
    functions.sort_by(|a, b| {
        b.complexity
            .cmp(&a.complexity)
            .then_with(|| b.loc.cmp(&a.loc))
            .then_with(|| a.line1.cmp(&b.line1))
            .then_with(|| a.name.cmp(&b.name))
    });
    let functions = functions
        .into_iter()
        .map(|function| HealthFunctionSummary {
            name: function.name,
            line1: function.line1,
            complexity: function.complexity,
            nesting: function.nesting,
            loc: function.loc,
            maintainability: function.maintainability,
            maintainability_index: function.maintainability,
        })
        .collect();
    let mut refactorings = refact_codehealth::refactoring::rank(
        refact_codehealth::refactoring::suggest_refactorings(&lang, &text),
    );
    refactorings.sort_by(|a, b| {
        b.impact
            .total_cmp(&a.impact)
            .then_with(|| a.line.cmp(&b.line))
    });

    Ok(HealthFileCore {
        path,
        lang,
        functions,
        max_complexity: health.max_complexity,
        avg_maintainability: health.avg_maintainability,
        duplication_pct,
        dry_violation,
        findings,
        refactorings,
    })
}

fn health_analysis_from_core(core: HealthFileCore, cache_hit: bool) -> HealthFileAnalysis {
    let plain_findings = core
        .findings
        .iter()
        .map(|finding| finding.finding.clone())
        .collect::<Vec<_>>();
    let (score, impacts) = refact_codehealth::scoring::score_file_with_impacts(&plain_findings);
    let mut health_impact = impacts
        .into_iter()
        .filter_map(|impact| {
            core.findings
                .get(impact.index)
                .map(|finding| HealthImpactContributor {
                    biomarker: finding.finding.biomarker.clone(),
                    category: finding.finding.category.clone(),
                    dimension: finding.finding.dimension,
                    severity: finding.finding.severity,
                    line: finding.finding.line,
                    detail: finding.finding.detail.clone(),
                    deduction: impact.deduction,
                    capped: impact.capped,
                })
        })
        .collect::<Vec<_>>();
    health_impact.sort_by(|a, b| {
        b.deduction
            .total_cmp(&a.deduction)
            .then_with(|| a.biomarker.cmp(&b.biomarker))
            .then_with(|| a.line.cmp(&b.line))
    });
    health_impact.truncate(5);
    let impact_summaries = health_impact
        .iter()
        .map(health_impact_summary)
        .collect::<Vec<_>>();
    let function_count = core.functions.len();
    let biomarker_count = core.findings.len();
    let refactoring_count = core.refactorings.len();

    HealthFileAnalysis {
        path: core.path,
        lang: core.lang,
        grade: refact_codehealth::scoring::grade(score.defect),
        maintainability_index: core.avg_maintainability,
        max_complexity: core.max_complexity,
        avg_maintainability: core.avg_maintainability,
        function_count,
        duplication_pct: core.duplication_pct,
        dry_violation: core.dry_violation,
        defect_score: score.defect,
        maintainability_score: score.maintainability,
        performance_score: score.performance,
        biomarker_count,
        refactoring_count,
        functions: core.functions,
        findings: core.findings,
        health_impact,
        impact_summaries,
        refactorings: core.refactorings,
        cache_hit,
    }
}

fn health_impact_summary(impact: &HealthImpactContributor) -> String {
    format!(
        "−{:.1} {}: {} — {}",
        impact.deduction,
        health_dimension_label(impact.dimension),
        impact.biomarker,
        impact.detail
    )
}

fn health_dimension_label(dimension: refact_codehealth::biomarkers::Dimension) -> &'static str {
    match dimension {
        refact_codehealth::biomarkers::Dimension::Defect => "defect",
        refact_codehealth::biomarkers::Dimension::Maintainability => "maintainability",
        refact_codehealth::biomarkers::Dimension::Performance => "performance",
    }
}

fn health_dry_findings(
    path: &str,
    lang: &str,
    text: &str,
    duplication_pct: f64,
) -> Vec<refact_codehealth::biomarkers::Finding> {
    let clones = refact_codehealth::duplication::detect_clones(lang, text)
        .into_iter()
        .map(|clone| refact_codehealth::dry::DryClonePair {
            file_a: path.to_string(),
            a_start_line: clone.line_a,
            a_line_count: clone
                .a_end_line
                .saturating_sub(clone.line_a)
                .saturating_add(1),
            file_b: path.to_string(),
            b_start_line: clone.line_b,
            b_line_count: clone
                .b_end_line
                .saturating_sub(clone.line_b)
                .saturating_add(1),
            co_change_count: 0,
        })
        .collect();
    let input = refact_codehealth::dry::DryInput {
        file_path: path.to_string(),
        duplication_pct,
        clones,
    };
    refact_codehealth::dry::dry_violation(&input)
}

async fn health_git_findings(
    path: &str,
    ctx: &HealthAnalysisContext<'_>,
    health: &refact_codehealth::FileHealth,
) -> Result<Vec<refact_codehealth::biomarkers::Finding>, String> {
    let (Some(intel), Some(repo_root)) = (ctx.intel, ctx.repo_root) else {
        return Ok(Vec::new());
    };
    let repo_root_str = repo_root.to_string_lossy();
    let path_key = refact_git_intel::paths::repo_relative_or_basename(path, &repo_root_str);
    let ranges = if ctx.git_function_fact_paths.contains(&path_key) {
        health
            .functions
            .iter()
            .map(|function| {
                let line2 = function
                    .line1
                    .saturating_add(function.loc as usize)
                    .saturating_sub(1)
                    .max(function.line1);
                (function.name.clone(), function.line1, line2)
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let meta = build_git_meta(
        intel,
        repo_root,
        &path_key,
        ctx.service,
        (!ranges.is_empty()).then_some(ranges.as_slice()),
    )
    .await;
    Ok(refact_codehealth::git_biomarkers::git_biomarkers(&meta))
}

fn health_coverage_findings(
    path: &str,
    coverage: Option<&refact_codehealth::coverage::CoverageReport>,
    repo_root: Option<&Path>,
    intel: Option<&refact_git_intel::GitIntel>,
    max_complexity: u32,
) -> Vec<refact_codehealth::biomarkers::Finding> {
    let Some(coverage) = coverage else {
        return Vec::new();
    };
    let Some(file) = coverage_file_for_path(coverage, path, repo_root) else {
        return Vec::new();
    };
    let mut complexity_by_file = HashMap::new();
    complexity_by_file.insert(file.path.clone(), max_complexity);
    let mut churn_by_file = HashMap::new();
    if let Some(churn) = health_churn_for_path(intel, &file.path, repo_root) {
        churn_by_file.insert(file.path.clone(), churn);
    }
    let report = refact_codehealth::coverage::CoverageReport {
        format: coverage.format.clone(),
        files: vec![file],
    };
    refact_codehealth::coverage_biomarkers::coverage_biomarkers(
        &report,
        &complexity_by_file,
        &churn_by_file,
    )
}

fn coverage_file_for_path(
    coverage: &refact_codehealth::coverage::CoverageReport,
    path: &str,
    repo_root: Option<&Path>,
) -> Option<refact_codehealth::coverage::FileCoverage> {
    let matches = coverage
        .files
        .iter()
        .filter(|file| health_paths_refer_to_same_file(&file.path, path, repo_root))
        .cloned()
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

fn health_coverage_signature(
    coverage: Option<&refact_codehealth::coverage::CoverageReport>,
    path: &str,
    repo_root: Option<&Path>,
) -> Option<String> {
    coverage.map(|coverage| {
        let file = coverage_file_for_path(coverage, path, repo_root);
        serde_json::to_string(&(coverage.format.as_str(), file))
            .map(hash_string)
            .unwrap_or_else(|_| "coverage-unserializable".to_string())
    })
}

fn health_churn_for_path(
    intel: Option<&refact_git_intel::GitIntel>,
    path: &str,
    repo_root: Option<&Path>,
) -> Option<u32> {
    let intel = intel?;
    intel.file_churn.get(path).copied().or_else(|| {
        intel
            .file_churn
            .iter()
            .find(|(candidate, _)| health_paths_refer_to_same_file(candidate, path, repo_root))
            .map(|(_, churn)| *churn)
    })
}

fn health_trend_findings_for_path(
    findings: &[refact_codehealth::biomarkers::Finding],
    path: &str,
    repo_root: Option<&Path>,
) -> Vec<refact_codehealth::biomarkers::Finding> {
    findings
        .iter()
        .filter(|finding| {
            health_trend_path(&finding.detail).is_some_and(|trend_path| {
                health_paths_refer_to_same_file(trend_path, path, repo_root)
            })
        })
        .cloned()
        .collect()
}

fn health_trend_signature(
    findings: &[refact_codehealth::biomarkers::Finding],
    path: &str,
    repo_root: Option<&Path>,
) -> String {
    let relevant = health_trend_findings_for_path(findings, path, repo_root);
    if relevant.is_empty() {
        return String::new();
    }
    serde_json::to_string(&relevant)
        .map(hash_string)
        .unwrap_or_else(|_| "trend-unserializable".to_string())
}

fn health_trend_path(detail: &str) -> Option<&str> {
    detail.split_once(':').map(|(path, _)| path.trim())
}

pub(crate) fn health_paths_refer_to_same_file(a: &str, b: &str, repo_root: Option<&Path>) -> bool {
    match repo_root {
        Some(repo_root) => {
            refact_git_intel::paths::paths_refer_to_same_file(a, b, &repo_root.to_string_lossy())
        }
        None => {
            refact_git_intel::paths::normalize_separators(a)
                == refact_git_intel::paths::normalize_separators(b)
        }
    }
}

pub(crate) fn health_stored_path_for_request(
    paths: impl IntoIterator<Item = String>,
    requested: &str,
    repo_root: Option<&Path>,
) -> Option<String> {
    let matches = paths
        .into_iter()
        .filter(|path| health_paths_refer_to_same_file(path, requested, repo_root))
        .collect::<BTreeSet<_>>();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

fn sort_health_findings(findings: &mut [refact_codehealth::biomarkers::Finding]) {
    findings.sort_by(|a, b| {
        health_severity_rank(a.severity)
            .cmp(&health_severity_rank(b.severity))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.biomarker.cmp(&b.biomarker))
            .then_with(|| a.detail.cmp(&b.detail))
    });
}

fn health_severity_rank(severity: refact_codehealth::biomarkers::Severity) -> u8 {
    match severity {
        refact_codehealth::biomarkers::Severity::Critical => 0,
        refact_codehealth::biomarkers::Severity::High => 1,
        refact_codehealth::biomarkers::Severity::Medium => 2,
        refact_codehealth::biomarkers::Severity::Low => 3,
    }
}

#[cfg(test)]
fn reset_health_cache_for_tests() {
    *health_analysis_cache().lock().unwrap() = HealthAnalysisCache::default();
}

#[cfg(test)]
fn health_cache_stats_for_tests() -> (usize, usize) {
    let cache = health_analysis_cache().lock().unwrap();
    (cache.hits, cache.misses)
}

pub(crate) async fn build_git_meta(
    intel: &refact_git_intel::GitIntel,
    repo_root: &Path,
    path_repo_relative: &str,
    service: Option<&Arc<refact_codegraph::CodeGraphService>>,
    function_ranges: Option<&[(String, usize, usize)]>,
) -> refact_codehealth::git_biomarkers::GitMeta {
    let now_ts = current_unix_ts();
    let entropy_map = intel.change_entropy();
    let (added_90d, deleted_90d) = intel.lines_in_window(path_repo_relative, now_ts, 90);
    let (primary_owner_name, primary_owner_commit_pct) = intel.primary_owner(path_repo_relative);
    let (recent_owner_name, recent_owner_commit_pct) =
        intel.recent_owner(path_repo_relative, now_ts, 90);
    let functions = match function_ranges {
        Some(ranges) => build_function_git_facts(repo_root, path_repo_relative, ranges, now_ts),
        None => Vec::new(),
    };
    let repo_function_mod_p80 = function_mod_p80(&functions);
    let import_edges = match service {
        Some(service) => git_meta_import_edges(service, repo_root, path_repo_relative)
            .await
            .unwrap_or_default(),
        None => HashSet::new(),
    };

    refact_codehealth::git_biomarkers::GitMeta {
        file_path: path_repo_relative.to_string(),
        change_entropy: entropy_map.get(path_repo_relative).copied().unwrap_or(0.0),
        change_entropy_pct: intel.change_entropy_pct(path_repo_relative),
        commit_count_90d: intel.commit_count_in_window(path_repo_relative, now_ts, 90),
        commit_count_total: intel
            .file_churn
            .get(path_repo_relative)
            .copied()
            .unwrap_or(0),
        is_hotspot: intel.is_hotspot_file(path_repo_relative, GIT_META_FUNCTION_FACT_TOP_K),
        is_stable: file_is_stable(intel, path_repo_relative, now_ts),
        churn_percentile: intel.churn_percentile(path_repo_relative),
        lines_added_90d: added_90d,
        lines_deleted_90d: deleted_90d,
        nloc: nloc_for_file(repo_root, path_repo_relative),
        contributor_count: intel
            .file_authors
            .get(path_repo_relative)
            .map(|authors| authors.len().min(u32::MAX as usize) as u32)
            .unwrap_or(0),
        primary_owner_commit_pct,
        primary_owner_name,
        recent_owner_name,
        recent_owner_commit_pct,
        bus_factor: intel.bus_factor(path_repo_relative) as u32,
        prior_defect_count: intel.prior_defects(path_repo_relative),
        repo_active_contributors_90d: Some(intel.active_contributors_in_window(now_ts, 90)),
        repo_function_mod_p80,
        co_change_partners: intel
            .co_change_partners(path_repo_relative, 1)
            .into_iter()
            .map(
                |(path, count)| refact_codehealth::git_biomarkers::CoChangePartner {
                    path,
                    co_change_count: count as f64,
                },
            )
            .collect(),
        top_authors: intel
            .ownership(path_repo_relative)
            .into_iter()
            .map(|owner| (owner.author, owner.commits))
            .collect(),
        functions,
        repo_commit_counts: git_churn_by_file(intel),
        import_edges,
    }
}

pub(crate) async fn build_git_risk_assembly(
    intel: &refact_git_intel::GitIntel,
    repo_root: &Path,
    service: Option<&Arc<refact_codegraph::CodeGraphService>>,
    limit: usize,
    filter: Option<&str>,
) -> GitRiskAssembly {
    let now_ts = current_unix_ts();
    let all_hotspots = intel.hotspots(intel.file_churn.len().max(1));
    let mut selected = all_hotspots
        .into_iter()
        .filter(|hotspot| {
            filter.is_none_or(|filter| path_matches_git_filter(&hotspot.path, filter))
        })
        .take(limit)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        if let Some(filter) = filter {
            selected = intel
                .file_churn
                .keys()
                .filter(|path| path_matches_git_filter(path, filter))
                .take(limit)
                .map(|path| refact_git_intel::Hotspot {
                    path: path.clone(),
                    churn: intel.file_churn.get(path).copied().unwrap_or(0),
                })
                .collect();
        }
    }
    let temporal_map: HashMap<String, f64> = intel
        .temporal_hotspots(now_ts, intel.file_churn.len().max(1))
        .into_iter()
        .collect();
    let mut files = Vec::new();
    for (index, hotspot) in selected.iter().enumerate() {
        let ranges = if index < GIT_META_FUNCTION_FACT_TOP_K {
            function_ranges_from_disk(repo_root, &hotspot.path)
        } else {
            Vec::new()
        };
        let mut meta = build_git_meta(
            intel,
            repo_root,
            &hotspot.path,
            service,
            (!ranges.is_empty()).then_some(ranges.as_slice()),
        )
        .await;
        meta.is_hotspot = true;
        let findings = refact_codehealth::git_biomarkers::git_biomarkers(&meta);
        files.push(GitRiskFileAssembly {
            temporal_score: temporal_map.get(&hotspot.path).copied().unwrap_or(0.0),
            churn_risk: intel.churn_risk(&hotspot.path),
            meta,
            findings,
        });
    }
    let p80 = function_mod_p80(
        &files
            .iter()
            .flat_map(|file| file.meta.functions.iter().cloned())
            .collect::<Vec<_>>(),
    );
    if p80.is_some() {
        for file in &mut files {
            file.meta.repo_function_mod_p80 = p80;
            file.findings = refact_codehealth::git_biomarkers::git_biomarkers(&file.meta);
        }
    }

    GitRiskAssembly {
        files,
        recent_commit_risks: recent_commit_risk_summaries(intel),
    }
}

fn dead_code_path_matches(path: &str, filter: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if normalized == filter || normalized.ends_with(filter) {
        return true;
    }
    let basename = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    basename == filter
}

fn file_is_stable(intel: &refact_git_intel::GitIntel, file: &str, now_ts: i64) -> bool {
    let last_ts = intel
        .commit_records
        .iter()
        .filter(|commit| commit.files.iter().any(|(path, _, _)| path == file))
        .map(|commit| commit.ts)
        .max();
    last_ts.is_some_and(|ts| now_ts.saturating_sub(ts) / 86_400 > GIT_STABLE_AGE_DAYS)
}

fn nloc_for_file(repo_root: &Path, path_repo_relative: &str) -> u32 {
    std::fs::read_to_string(repo_root.join(path_repo_relative))
        .map(|text| {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .count()
                .min(u32::MAX as usize) as u32
        })
        .unwrap_or(0)
}

fn function_ranges_from_disk(
    repo_root: &Path,
    path_repo_relative: &str,
) -> Vec<(String, usize, usize)> {
    let Ok(text) = std::fs::read_to_string(repo_root.join(path_repo_relative)) else {
        return Vec::new();
    };
    let lang = refact_codegraph::lang_from_path(path_repo_relative);
    refact_codehealth::analyze(lang, &text)
        .functions
        .into_iter()
        .map(|function| {
            let line2 = function
                .line1
                .saturating_add(function.loc as usize)
                .saturating_sub(1)
                .max(function.line1);
            (function.name, function.line1, line2)
        })
        .collect()
}

fn build_function_git_facts(
    repo_root: &Path,
    path_repo_relative: &str,
    ranges: &[(String, usize, usize)],
    now_ts: i64,
) -> Vec<refact_codehealth::git_biomarkers::FunctionGitFacts> {
    let range_facts = refact_git_intel::blame::range_facts(repo_root, path_repo_relative, ranges)
        .unwrap_or_default();
    let health_by_range = function_health_by_range(repo_root, path_repo_relative);
    range_facts
        .into_iter()
        .map(|fact| {
            let health = health_by_range
                .get(&(fact.name.clone(), fact.line1))
                .or_else(|| health_by_range.get(&(fact.name.clone(), 0)));
            refact_codehealth::git_biomarkers::FunctionGitFacts {
                name: fact.name,
                median_age_days: age_days_from_ts(now_ts, fact.last_modified_ts),
                recent_mod_count: fact.author_count,
                modification_count: fact.commit_count,
                ccn: health.map(|(ccn, _)| *ccn).unwrap_or(0),
                max_nesting: health.map(|(_, nesting)| *nesting).unwrap_or(0),
            }
        })
        .collect()
}

fn function_health_by_range(
    repo_root: &Path,
    path_repo_relative: &str,
) -> HashMap<(String, usize), (u32, u32)> {
    let Ok(text) = std::fs::read_to_string(repo_root.join(path_repo_relative)) else {
        return HashMap::new();
    };
    let lang = refact_codegraph::lang_from_path(path_repo_relative);
    let mut out = HashMap::new();
    for function in refact_codehealth::analyze(lang, &text).functions {
        out.insert(
            (function.name.clone(), function.line1),
            (function.complexity, function.nesting),
        );
        out.entry((function.name, 0))
            .or_insert((function.complexity, function.nesting));
    }
    out
}

fn age_days_from_ts(now_ts: i64, ts: i64) -> u32 {
    if ts <= 0 {
        return 0;
    }
    let days = now_ts.saturating_sub(ts).max(0) / 86_400;
    days.min(u32::MAX as i64) as u32
}

fn function_mod_p80(
    functions: &[refact_codehealth::git_biomarkers::FunctionGitFacts],
) -> Option<u32> {
    if functions.is_empty() {
        return None;
    }
    let mut counts = functions
        .iter()
        .map(|function| function.modification_count)
        .collect::<Vec<_>>();
    counts.sort_unstable();
    let index = ((counts.len() as f64 * 0.8).ceil() as usize).saturating_sub(1);
    counts.get(index).copied()
}

async fn git_meta_import_edges(
    service: &Arc<refact_codegraph::CodeGraphService>,
    repo_root: &Path,
    path_repo_relative: &str,
) -> Result<HashSet<String>, String> {
    let cached = service.cached_graph_analytics().await?;
    let repo_root = repo_root.to_string_lossy();
    let mut path_by_id = HashMap::new();
    let mut source_paths = HashSet::new();
    for (id, _name, path) in &cached.data.nodes {
        path_by_id.insert(*id, path.clone());
        if refact_git_intel::paths::paths_refer_to_same_file(path, path_repo_relative, &repo_root) {
            source_paths.insert(path.clone());
        }
    }

    let mut out = HashSet::new();
    for (src, dst, kind) in &cached.data.edges {
        if kind != "calls" && kind != "imports" {
            continue;
        }
        let Some(src_path) = path_by_id.get(src) else {
            continue;
        };
        if !source_paths.contains(src_path) {
            continue;
        }
        let Some(dst_path) = path_by_id.get(dst) else {
            continue;
        };
        if refact_git_intel::paths::paths_refer_to_same_file(src_path, dst_path, &repo_root) {
            continue;
        }
        out.insert(refact_git_intel::paths::repo_relative_or_basename(
            dst_path, &repo_root,
        ));
    }
    Ok(out)
}

fn path_matches_git_filter(path: &str, filter: &str) -> bool {
    let normalized = refact_git_intel::paths::normalize_separators(path);
    normalized == filter
        || normalized.ends_with(filter)
        || normalized.rsplit('/').next() == Some(filter)
}

fn recent_commit_risk_summaries(
    intel: &refact_git_intel::GitIntel,
) -> Vec<RecentCommitRiskSummary> {
    intel
        .recent_commit_risks(GIT_RISK_RECENT_COMMITS)
        .into_iter()
        .take(GIT_RISK_RECENT_COMMIT_LIMIT)
        .map(|risk| RecentCommitRiskSummary {
            sha: truncate_chars(&risk.commit_id, 12),
            summary: truncate_chars(&risk.summary, 80),
            risk: risk.risk,
            top_factor_names: commit_risk_factor_names(&risk.inputs),
        })
        .collect()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

fn commit_risk_factor_names(inputs: &refact_git_intel::change_risk::RiskInputs) -> Vec<String> {
    let mut factors = [
        ("lines_added", inputs.la),
        ("lines_deleted", inputs.ld),
        ("files", inputs.nf),
        ("directories", inputs.nd),
        ("subsystems", inputs.ns),
        ("entropy", inputs.entropy * 100.0),
        ("low_experience", 100.0 / (inputs.exp + 1.0)),
    ];
    factors.sort_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    factors
        .into_iter()
        .filter(|(_, score)| *score > 0.0)
        .take(3)
        .map(|(name, _)| name.to_string())
        .collect()
}

pub(crate) fn git_risk_tool_output(
    intel: &refact_git_intel::GitIntel,
    assembly: &GitRiskAssembly,
) -> String {
    let mut msg = format!(
        "Git risk (over {} commits) — recency-weighted hotspots:\n",
        intel.commits_analyzed
    );
    for file in &assembly.files {
        let meta = &file.meta;
        let mut flags = Vec::new();
        if intel.ownership_risk(&meta.file_path) {
            flags.push("ownership-risk");
        }
        if intel.knowledge_loss(&meta.file_path) {
            flags.push("knowledge-loss");
        }
        if meta.prior_defect_count > 0 {
            flags.push("prior-defect");
        }
        msg.push_str(&format!(
            "  churn={} risk={:.2} temporal={:.2} bus_factor={} {}{}\n",
            meta.commit_count_total,
            file.churn_risk,
            file.temporal_score,
            meta.bus_factor,
            meta.file_path,
            if flags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", flags.join(", "))
            }
        ));
    }
    let pairs = intel.co_change_pairs(2);
    if !pairs.is_empty() {
        msg.push_str("\nFrequently co-changed files:\n");
        for ((a, b), c) in pairs.iter().take(10) {
            msg.push_str(&format!("  {}x  {} <-> {}\n", c, a, b));
        }
    }
    let agent_pct = intel.agent_authored_pct();
    if agent_pct > 0.0 {
        msg.push_str(&format!(
            "\nAI/agent authorship: {:.0}% of analyzed commits\n",
            agent_pct * 100.0
        ));
    }
    let coupling = refact_git_intel::coupling::build_coupling_graph(intel, 8);
    if !coupling.edges.is_empty() {
        msg.push_str("\nStrongest coupling (normalized):\n");
        for e in coupling.edges.iter().take(8) {
            msg.push_str(&format!(
                "  strength={:.2} ({}x)  {} <-> {}\n",
                e.strength, e.co_changes, e.a, e.b
            ));
        }
    }
    let hotspot_paths: Vec<String> = assembly
        .files
        .iter()
        .map(|file| file.meta.file_path.clone())
        .collect();
    let reviewers = refact_git_intel::coupling::reviewer_suggestions(intel, &hotspot_paths, 5);
    if !reviewers.is_empty() {
        msg.push_str("\nSuggested reviewers (by ownership of hot files + co-change neighbors):\n");
        for (author, score) in &reviewers {
            msg.push_str(&format!("  {} (score {:.2})\n", author, score));
        }
    }
    let mut biomarker_lines = Vec::new();
    for file in &assembly.files {
        for f in &file.findings {
            biomarker_lines.push(format!(
                "  {} [{:?}] {} — {}",
                f.biomarker, f.severity, file.meta.file_path, f.detail
            ));
        }
    }
    if !biomarker_lines.is_empty() {
        msg.push_str("\nGit-driven health biomarkers:\n");
        for line in biomarker_lines.iter().take(20) {
            msg.push_str(line);
            msg.push('\n');
        }
    }
    if !assembly.recent_commit_risks.is_empty() {
        msg.push_str("\nRecent commit change-risk (Kamei):\n");
        for risk in &assembly.recent_commit_risks {
            msg.push_str(&format!(
                "  {:.2} {} {} [{}]\n",
                risk.risk,
                risk.sha,
                risk.summary,
                risk.top_factor_names.join(", ")
            ));
        }
    }
    msg
}

pub struct ToolCodegraphOverview {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolCodegraphOverview {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.app.gcx.clone();
        let service = gcx
            .codegraph
            .lock()
            .await
            .clone()
            .ok_or_else(|| "codegraph is not available".to_string())?;
        let readiness = service.index_readiness().await?;
        let index_state = pr_blast_index_state(&readiness);
        let cached = service.cached_graph_analytics().await?;
        let overview = cached.analytics.overview.truncated(15);
        let mut msg = String::new();
        if let Some(warning) = pr_blast_partial_warning(&index_state) {
            msg.push_str(&warning);
            msg.push('\n');
        }
        msg.push_str(&format!(
            "Code graph overview:\n  nodes: {}\n  edges: {}\n  connected components: {}\n  strongly-connected components: {} (largest {})\n",
            overview.node_count,
            overview.edge_count,
            overview.component_count,
            overview.scc_count,
            overview.largest_scc
        ));
        msg.push_str(&format!(
            "Index state: queued={} cross_file_edges={} cross_file_ready={} partial={}\n",
            index_state.queued,
            index_state.cross_file_edges,
            index_state.cross_file_ready,
            !index_state.cross_file_ready
        ));
        msg.push_str("\nMost central symbols (PageRank):\n");
        for entry in &overview.top_pagerank {
            msg.push_str(&format!(
                "  {:.4}  {}\n",
                entry.score,
                symbol_score_label(entry)
            ));
        }
        msg.push_str("\nKey connectors (betweenness centrality):\n");
        for entry in overview
            .top_betweenness
            .iter()
            .filter(|entry| entry.score > 0.0)
        {
            msg.push_str(&format!(
                "  {:.2}  {}\n",
                entry.score,
                symbol_score_label(entry)
            ));
        }
        let mut communities = cached.communities.clone();
        communities.sort_by(|a, b| b.members.len().cmp(&a.members.len()));
        if !communities.is_empty() {
            msg.push_str(&format!("\nModule communities ({}):\n", communities.len()));
            for c in communities.iter().take(8) {
                msg.push_str(&format!(
                    "  {} ({} members, cohesion {:.2})\n",
                    c.label,
                    c.members.len(),
                    c.cohesion
                ));
            }
        }
        if let Ok(flows) = service.execution_flows(5).await {
            if !flows.is_empty() {
                msg.push_str("\nExecution flows (entry points):\n");
                for f in &flows {
                    msg.push_str(&format!(
                        "  {} reaches {} nodes (depth {})\n",
                        f.entry, f.reached, f.depth
                    ));
                }
            }
        }
        let mut dead = cached.dead_code.clone();
        dead.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if !dead.is_empty() {
            msg.push_str(&format!("\nLikely dead code ({}):\n", dead.len()));
            for d in dead.iter().take(15) {
                msg.push_str(&format!(
                    "  {} @ {} ({}, confidence {:.2})\n",
                    d.name, d.path, d.reason, d.confidence
                ));
            }
        }
        let all_files = service.all_files_with_text().await.unwrap_or_default();
        let centrality = cached.analytics.file_centrality.truncated(100);
        let pr: HashMap<String, f64> = centrality.top_pagerank.iter().cloned().collect();
        let bt: HashMap<String, f64> = centrality.top_betweenness.iter().cloned().collect();
        let candidates: Vec<(String, f64, f64)> = all_files
            .iter()
            .map(|(p, _)| {
                let pagerank = pr.get(p).copied().unwrap_or(0.0);
                let betweenness = bt.get(p).copied().unwrap_or(0.0);
                (p.clone(), pagerank, betweenness)
            })
            .collect();
        let stems = refact_codewiki::entry_points::default_conventional_stems();
        let ranked = refact_codewiki::entry_points::rank_entry_points(&candidates, &stems);
        if !ranked.is_empty() {
            msg.push_str("\nLikely entry points (conventional name + shallow depth):\n");
            for p in ranked.iter().take(10) {
                msg.push_str(&format!("  {}\n", p));
            }
        }
        let mut api_files: Vec<String> = all_files
            .iter()
            .filter(|(p, t)| {
                let lang = refact_codegraph::lang_from_path(p);
                if !matches!(lang, "python" | "py" | "csharp" | "cs") {
                    return false;
                }
                refact_codewiki::api_contract::detect_api_contract(
                    &refact_codewiki::parsed_file::build_parsed_file(lang, t),
                )
            })
            .map(|(p, _)| p.clone())
            .collect();
        api_files.sort();
        if !api_files.is_empty() {
            msg.push_str(&format!("\nAPI-contract files ({}):\n", api_files.len()));
            for p in api_files.iter().take(15) {
                msg.push_str(&format!("  {}\n", p));
            }
        }
        Ok((false, tool_message(tool_call_id, msg)))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "codegraph_overview".to_string(),
            display_name: "Code Graph Overview".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Project-wide CodeGraph overview from the generation-keyed analytics cache: readiness warning, node/edge counts, SCCs, PageRank/betweenness symbols with paths, communities, entry points, API-contract files, and likely dead code.".to_string(),
            input_schema: json_schema_from_params(&[], &[]),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct HealthFinding {
    #[serde(flatten)]
    pub(crate) finding: refact_codehealth::biomarkers::Finding,
    pub(crate) hot_path: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FunctionKey {
    name: String,
    line1: usize,
    line2: usize,
}

#[derive(Default)]
struct HealthGraphContext {
    fan_in: HashMap<FunctionKey, usize>,
    hot_path: HashSet<FunctionKey>,
}

pub(crate) async fn enrich_health_findings(
    service: Option<&Arc<refact_codegraph::CodeGraphService>>,
    file_path: &str,
    functions: &[refact_codehealth::FunctionHealth],
    findings: Vec<refact_codehealth::biomarkers::Finding>,
) -> Vec<HealthFinding> {
    let graph = match service {
        Some(service) => service.cached_graph_analytics().await.ok(),
        None => None,
    };
    enrich_health_findings_with_graph(
        graph.as_ref().map(|cached| &cached.data),
        file_path,
        functions,
        findings,
    )
}

pub(crate) fn enrich_health_findings_with_graph(
    graph: Option<&refact_codegraph::analytics::GraphData>,
    file_path: &str,
    functions: &[refact_codehealth::FunctionHealth],
    findings: Vec<refact_codehealth::biomarkers::Finding>,
) -> Vec<HealthFinding> {
    let context = graph.map(|graph| health_graph_context(graph, file_path, functions));
    findings
        .into_iter()
        .map(|finding| enrich_health_finding(finding, functions, context.as_ref()))
        .collect()
}

fn enrich_health_finding(
    mut finding: refact_codehealth::biomarkers::Finding,
    functions: &[refact_codehealth::FunctionHealth],
    context: Option<&HealthGraphContext>,
) -> HealthFinding {
    let mut hot_path = false;
    if let (Some(context), Some(function)) = (context, containing_function(functions, finding.line))
    {
        let key = function_key(function);
        if finding.biomarker == "brain_method" {
            if let Some(fan_in) = context.fan_in.get(&key).copied() {
                if fan_in == 0 {
                    finding.severity = downgrade_health_severity(finding.severity);
                    finding.detail.push_str(" (low fan-in)");
                } else if fan_in >= 5 {
                    finding
                        .detail
                        .push_str(&format!(" (called from {fan_in} sites)"));
                }
            }
        }
        if finding.dimension == refact_codehealth::biomarkers::Dimension::Performance {
            hot_path = context.hot_path.contains(&key);
        }
    }
    HealthFinding { finding, hot_path }
}

fn health_graph_context(
    graph: &refact_codegraph::analytics::GraphData,
    file_path: &str,
    functions: &[refact_codehealth::FunctionHealth],
) -> HealthGraphContext {
    let mut node_ids_by_key = HashMap::<FunctionKey, BTreeSet<i64>>::new();
    let mut key_by_node_id = HashMap::<i64, Vec<FunctionKey>>::new();
    for function in functions {
        let key = function_key(function);
        for (id, name, path) in &graph.nodes {
            if graph_path_matches(path, file_path) && graph_name_matches(name, &function.name) {
                node_ids_by_key.entry(key.clone()).or_default().insert(*id);
                key_by_node_id.entry(*id).or_default().push(key.clone());
            }
        }
    }

    let mut sources_by_key = HashMap::<FunctionKey, BTreeSet<i64>>::new();
    for (src, dst, kind) in &graph.edges {
        if kind != "calls" {
            continue;
        }
        if let Some(keys) = key_by_node_id.get(dst) {
            for key in keys {
                sources_by_key.entry(key.clone()).or_default().insert(*src);
            }
        }
    }

    let hot_node_ids = hot_path_node_ids(graph, 3);
    let hot_path = node_ids_by_key
        .iter()
        .filter_map(|(key, ids)| {
            ids.iter()
                .any(|id| hot_node_ids.contains(id))
                .then(|| key.clone())
        })
        .collect();

    HealthGraphContext {
        fan_in: node_ids_by_key
            .into_keys()
            .map(|key| {
                let fan_in = sources_by_key.get(&key).map(BTreeSet::len).unwrap_or(0);
                (key, fan_in)
            })
            .collect(),
        hot_path,
    }
}

fn hot_path_node_ids(
    graph: &refact_codegraph::analytics::GraphData,
    max_depth: usize,
) -> BTreeSet<i64> {
    let mut out = HashMap::<i64, Vec<i64>>::new();
    let mut roots = BTreeSet::new();
    for (src, dst, kind) in &graph.edges {
        if !matches!(kind.as_str(), "calls" | "route_handler") {
            continue;
        }
        out.entry(*src).or_default().push(*dst);
        if kind == "route_handler" {
            roots.insert(*src);
        }
    }
    for (id, name, _) in &graph.nodes {
        if graph_entry_name(name) {
            roots.insert(*id);
        }
    }

    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for root in roots {
        if seen.insert(root) {
            queue.push_back((root, 0));
        }
    }
    while let Some((id, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        if let Some(targets) = out.get(&id) {
            for target in targets {
                if seen.insert(*target) {
                    queue.push_back((*target, depth + 1));
                }
            }
        }
    }
    seen
}

fn containing_function(
    functions: &[refact_codehealth::FunctionHealth],
    line: usize,
) -> Option<&refact_codehealth::FunctionHealth> {
    functions
        .iter()
        .filter(|function| {
            let line2 = function_line2(function);
            function.line1 <= line && line <= line2
        })
        .max_by_key(|function| function.line1)
}

fn function_key(function: &refact_codehealth::FunctionHealth) -> FunctionKey {
    FunctionKey {
        name: function.name.clone(),
        line1: function.line1,
        line2: function_line2(function),
    }
}

fn function_line2(function: &refact_codehealth::FunctionHealth) -> usize {
    function
        .line1
        .saturating_add(function.loc as usize)
        .saturating_sub(1)
        .max(function.line1)
}

fn downgrade_health_severity(
    severity: refact_codehealth::biomarkers::Severity,
) -> refact_codehealth::biomarkers::Severity {
    match severity {
        refact_codehealth::biomarkers::Severity::Critical => {
            refact_codehealth::biomarkers::Severity::High
        }
        refact_codehealth::biomarkers::Severity::High => {
            refact_codehealth::biomarkers::Severity::Medium
        }
        refact_codehealth::biomarkers::Severity::Medium => {
            refact_codehealth::biomarkers::Severity::Low
        }
        refact_codehealth::biomarkers::Severity::Low => {
            refact_codehealth::biomarkers::Severity::Low
        }
    }
}

fn graph_path_matches(graph_path: &str, requested_path: &str) -> bool {
    // Exact or suffix path matches only. A bare-basename fallback is deliberately
    // NOT allowed here: enrichment (fan-in gates, hot-path flags) must never
    // borrow signals from a same-named file in another directory.
    let graph_path = graph_path.replace('\\', "/");
    let requested_path = requested_path.replace('\\', "/");
    graph_path == requested_path
        || graph_path.ends_with(&format!("/{requested_path}"))
        || requested_path.ends_with(&format!("/{graph_path}"))
}

fn graph_name_matches(graph_name: &str, function_name: &str) -> bool {
    graph_name == function_name
        || graph_name.ends_with(&format!("::{function_name}"))
        || graph_name.ends_with(&format!(".{function_name}"))
}

fn graph_entry_name(name: &str) -> bool {
    name == "main" || name.ends_with("::main") || name.ends_with(".main")
}

pub struct ToolDeadCode {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolDeadCode {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let limit = optional_usize_arg(args, "limit", DEAD_CODE_DEFAULT_LIMIT)?
            .clamp(1, DEAD_CODE_MAX_LIMIT);
        let min_confidence = optional_f64_arg(args, "min_confidence", 0.5)?.clamp(0.0, 1.0);
        let path = optional_string_arg(args, "path")?;
        let path = match path {
            Some(path) => Some(resolve_codegraph_filter_path(ccx.clone(), &path).await?),
            None => None,
        };
        let roots = codegraph_project_roots(ccx.clone()).await?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let report = dead_code_report_with_roots(
            gcx,
            limit,
            path.as_deref(),
            min_confidence,
            roots.as_ref().map(|roots| roots.git_root.as_path()),
            roots.as_ref().map(|roots| roots.indexed_root.as_path()),
        )
        .await?;
        Ok((
            false,
            tool_message(tool_call_id, dead_code_tool_text(&report)),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "dead_code".to_string(),
            display_name: "Dead Code".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Dead-code candidates from cached CodeGraph reachability: excludes override methods, build scripts, and shell setup/install entry points; adds git recency/churn confidence, path/min-confidence filters, and readiness warnings while the index is partial.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("limit", "integer", "Maximum number of candidates to return. Defaults to 50, max 500."),
                    (
                        "path",
                        "string",
                        "Optional workspace-relative, absolute current-project, active-worktree, or basename filter; accepts the same path forms as cat.",
                    ),
                    ("min_confidence", "number", "Minimum confidence threshold. Defaults to 0.5."),
                ],
                &[],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

fn dead_code_tool_text(report: &DeadCodeReport) -> String {
    let mut msg = String::new();
    if let Some(warning) = &report.warning {
        msg.push_str(warning);
        msg.push('\n');
    }
    msg.push_str(&format!(
        "Dead code candidates: {} shown of {} matching candidates.\n",
        report.entries.len(),
        report.total_candidates
    ));
    msg.push_str(&format!(
        "Index state: queued={} dirty_paths={} pending_refs={} cross_file_edges={} cross_file_ready={} partial={}\n",
        report.index_state.queued,
        report.index_state.dirty_paths,
        report.index_state.pending_refs,
        report.index_state.cross_file_edges,
        report.index_state.cross_file_ready,
        report.partial
    ));
    if report.entries.is_empty() {
        msg.push_str("\nNo dead-code candidates matched the filters.\n");
        return msg;
    }

    let mut grouped: Vec<(&str, Vec<&DeadCodeEntry>)> = Vec::new();
    for entry in &report.entries {
        if let Some((_, entries)) = grouped.iter_mut().find(|(path, _)| *path == entry.path) {
            entries.push(entry);
        } else {
            grouped.push((entry.path.as_str(), vec![entry]));
        }
    }
    for (path, entries) in grouped {
        msg.push_str(&format!("\n{}:\n", path));
        for entry in entries {
            msg.push_str(&format!(
                "  {:.2}  line {}  {} — {}; {}\n",
                entry.confidence, entry.line, entry.name, entry.reason, entry.git_recency
            ));
        }
    }
    msg
}

pub struct ToolCodeHealth {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolCodeHealth {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let requested_file_path = string_arg(args, "file_path")?;
        let resolved_file = resolve_codegraph_file(ccx.clone(), &requested_file_path).await?;
        let file_path = resolved_file.indexed_path;
        let gcx = ccx.lock().await.app.gcx.clone();
        let text = crate::files_in_workspace::get_file_text_from_memory_or_disk(
            gcx.clone(),
            &resolved_file.read_path,
        )
        .await?
        .to_string();
        let coverage = match args.get("coverage_file") {
            Some(Value::String(cov_path)) if !cov_path.trim().is_empty() => {
                let resolved_coverage = resolve_codegraph_file(ccx.clone(), cov_path).await?;
                let rope = crate::files_in_workspace::get_file_text_from_memory_or_disk(
                    gcx.clone(),
                    &resolved_coverage.read_path,
                )
                .await?;
                refact_codehealth::coverage::detect_and_parse(&rope.to_string())
            }
            _ => None,
        };
        let service = gcx.codegraph.lock().await.clone();
        let roots = codegraph_project_roots(ccx.clone()).await?;
        let repo_root = roots.as_ref().map(|roots| roots.indexed_root.as_path());
        let intel = roots
            .as_ref()
            .map(|roots| roots.git_root.as_path())
            .and_then(|dir| cached_mine_history(dir, GIT_HISTORY_MAX_COMMITS).ok());
        let snapshots = match service.as_deref() {
            Some(service) => load_health_snapshots(service).await,
            None => Vec::new(),
        };
        let trend_findings = refact_codehealth::trends::evaluate_trends(&snapshots);
        let git_function_fact_paths = top_git_function_fact_paths(intel.as_ref());
        let ctx = HealthAnalysisContext {
            repo_root,
            intel: intel.as_ref(),
            service: service.as_ref(),
            coverage: coverage.as_ref(),
            trend_findings: &trend_findings,
            git_function_fact_paths: &git_function_fact_paths,
        };
        let analysis = analyze_health_file_shared(file_path.clone(), text, &ctx).await?;
        if analysis.functions.is_empty() {
            return Ok((
                false,
                tool_message(
                    tool_call_id,
                    format!(
                        "No functions analyzed in `{requested_file_path}` (lang: {}).",
                        analysis.lang
                    ),
                ),
            ));
        }

        let mut msg = format!(
            "Code health for `{}` (MI (0-100) {:.1}, max complexity {}):\n",
            requested_file_path, analysis.maintainability_index, analysis.max_complexity
        );
        let category =
            refact_codewiki::well_known::file_category(&file_path, &analysis.lang, false);
        match refact_codewiki::well_known::well_known_role(&file_path) {
            Some(role) => msg.push_str(&format!("File role: {role} (category: {category})\n")),
            None => msg.push_str(&format!("File category: {category}\n")),
        }
        for f in analysis.functions.iter().take(30) {
            msg.push_str(&format!(
                "  {}:{}  complexity={} nesting={} loc={} MI (0-100)={:.0}\n",
                f.name, f.line1, f.complexity, f.nesting, f.loc, f.maintainability_index
            ));
        }
        if analysis.duplication_pct > 0.0 {
            msg.push_str(&format!(
                "\nDuplication: {:.0}% of tokens are in clones{}\n",
                analysis.duplication_pct * 100.0,
                if analysis.dry_violation {
                    " (DRY violation)"
                } else {
                    ""
                }
            ));
        }
        msg.push_str(&format!(
            "\nHealth score (1-10): defect={:.1} ({}) · maintainability signal (1-10)={:.1} · performance={:.1}\n",
            analysis.defect_score,
            analysis.grade,
            analysis.maintainability_score,
            analysis.performance_score
        ));
        if !analysis.health_impact.is_empty() {
            msg.push_str("Top health impact contributors:\n");
            for summary in &analysis.impact_summaries {
                msg.push_str("  ");
                msg.push_str(summary);
                msg.push('\n');
            }
        }
        if !analysis.findings.is_empty() {
            msg.push_str(&format!("Biomarkers ({}):\n", analysis.findings.len()));
            for fnd in analysis.findings.iter().take(15) {
                let marker = if fnd.hot_path { " 🔥 hot path" } else { "" };
                msg.push_str(&format!(
                    "  {}:{} {} [{:?}/{:?}] {}{}\n",
                    file_path,
                    fnd.finding.line,
                    fnd.finding.biomarker,
                    fnd.finding.severity,
                    fnd.finding.dimension,
                    fnd.finding.detail,
                    marker
                ));
            }
        }
        if !analysis.refactorings.is_empty() {
            msg.push_str("Refactoring targets:\n");
            for r in analysis.refactorings.iter().take(8) {
                msg.push_str(&format!(
                    "  {:?} @ {}:{} — {} (impact {:.1}, {} effort)\n",
                    r.kind, file_path, r.line, r.rationale, r.impact, r.effort
                ));
            }
        }
        if let Some(service) = service.as_ref() {
            if let Ok(cached) = service.cached_graph_analytics().await {
                let graph = refact_codewiki::graph_intelligence::CodeGraph {
                    nodes: cached
                        .data
                        .nodes
                        .iter()
                        .map(
                            |(id, _name, path)| refact_codewiki::graph_intelligence::GraphNode {
                                id: id.to_string(),
                                node_type: "symbol".to_string(),
                                file_path: path.clone(),
                            },
                        )
                        .collect(),
                    edges: cached
                        .data
                        .edges
                        .iter()
                        .map(
                            |(src, dst, kind)| refact_codewiki::graph_intelligence::GraphEdge {
                                src: src.to_string(),
                                dst: dst.to_string(),
                                edge_type: kind.clone(),
                            },
                        )
                        .collect(),
                };
                let stored_path = health_stored_path_for_request(
                    cached.data.nodes.iter().map(|(_, _, path)| path.clone()),
                    &file_path,
                    repo_root,
                );
                if let Some(stored_path) = stored_path {
                    let calls = refact_codewiki::graph_intelligence::extract_call_graph(
                        &stored_path,
                        &graph,
                    );
                    if !calls.is_empty() {
                        msg.push_str(&format!(
                            "\nCall graph ({} edges involving this file):\n",
                            calls.len()
                        ));
                        for c in calls.iter().take(12) {
                            msg.push_str(&format!("  {} -> {}\n", c.caller, c.callee));
                        }
                    }
                }
            }
        }
        if let Some(report) = coverage.as_ref() {
            if let Some(file) = coverage_file_for_path(report, &file_path, repo_root) {
                let single = refact_codehealth::coverage::CoverageReport {
                    format: report.format.clone(),
                    files: vec![file],
                };
                let (line_pct, branch_pct, below_50) =
                    refact_codehealth::coverage_biomarkers::coverage_summary(&single);
                msg.push_str(&format!(
                    "\nCoverage ({}): {:.0}% lines, {:.0}% branches, {} files below 50%\n",
                    single.format, line_pct, branch_pct, below_50
                ));
            }
        }
        if analysis.cache_hit {
            msg.push_str("\nWarm cache: served unchanged file analysis from cache.\n");
        }
        Ok((false, tool_message(tool_call_id, msg)))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "code_health".to_string(),
            display_name: "Code Health".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Deterministic per-file health: function complexity/nesting/LOC/MI, duplication, structural/git/coverage/trend/performance findings, hot-path and fan-in graph enrichment, cached unchanged-file analysis, 1-10 defect/maintainability/performance scores with A-F grade, health-impact contributors, and refactoring targets.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    (
                        "file_path",
                        "string",
                        "File to analyze; accepts the same workspace-relative, absolute current-project, and active-worktree path forms as cat.",
                    ),
                    (
                        "coverage_file",
                        "string",
                        "Optional LCOV/Cobertura/Clover report; accepts the same path forms as cat.",
                    ),
                ],
                &["file_path"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

pub struct ToolGitRisk {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolGitRisk {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let roots = codegraph_project_roots(ccx.clone())
            .await?
            .ok_or_else(|| "no project directory available".to_string())?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let intel = cached_mine_history(&roots.git_root, 1000)?;
        if intel.hotspots(1).is_empty() {
            return Ok((
                false,
                tool_message(tool_call_id, "No git history found.".to_string()),
            ));
        }
        let service = gcx.codegraph.lock().await.clone();
        let assembly =
            build_git_risk_assembly(&intel, &roots.indexed_root, service.as_ref(), 15, None).await;
        Ok((
            false,
            tool_message(tool_call_id, git_risk_tool_output(&intel, &assembly)),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "git_risk".to_string(),
            display_name: "Git Risk".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Git intelligence for the project: mined history summary, churn/temporal hotspots, ownership and bus-factor risk, co-change/coupling pairs, reviewer ownership hints, recent commit risk factors, and git-biomarker findings with function-level facts for the top hotspot files.".to_string(),
            input_schema: json_schema_from_params(&[], &[]),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

pub struct ToolCodeWhy {
    pub config_path: String,
}

const CODE_WHY_HISTORY_COMMITS: usize = 2000;
const CODE_WHY_SIGNIFICANT_COMMITS: usize = 500;
const CODE_WHY_SOURCE_LIMIT: usize = 800;
const CODE_WHY_MATCH_LIMIT: usize = 20;
const CODE_WHY_RELATED_PER_MATCH: usize = 4;

#[cfg(test)]
static CODE_WHY_FILE_READ_ATTEMPTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeWhySource {
    kind: String,
    text: String,
    reference: String,
    priority: u8,
    order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeWhyFileCandidate {
    relative: PathBuf,
    kind: &'static str,
    reference: String,
    priority: u8,
    order: usize,
}

#[derive(Debug, Clone)]
struct CodeWhySourceMeta {
    priority: u8,
    order: usize,
    reference: String,
    file: Option<CodeWhyFileCandidate>,
}

#[derive(Debug, Clone)]
struct CodeWhyCandidate {
    decision: refact_codewiki::Decision,
    source_rank: u8,
    source_ref: String,
}

#[derive(Debug, Clone)]
struct CodeWhyDecision {
    decision: refact_codewiki::Decision,
    source_rank: u8,
    source_ref: String,
}

#[derive(Debug, Clone)]
struct ScoredCodeWhyDecision {
    item: CodeWhyDecision,
    confidence: f64,
    verification: refact_codewiki::Verification,
}

fn code_why_priority(kind: &str) -> u8 {
    match kind {
        "adr" => 0,
        "changelog" => 1,
        "pr" => 2,
        _ => 3,
    }
}

fn short_oid(oid: Option<&String>, fallback: usize) -> String {
    oid.map(|oid| oid.chars().take(12).collect::<String>())
        .filter(|oid| !oid.is_empty())
        .unwrap_or_else(|| format!("commit:{fallback}"))
}

fn path_ref(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn merge_commit_has_body(message: &str) -> bool {
    let mut lines = message.lines();
    let first = lines.next().unwrap_or("").trim();
    if !(first.starts_with("Merge pull request") || first.starts_with("Merge branch")) {
        return false;
    }
    lines.any(|line| !line.trim().is_empty())
}

fn commit_prose_text(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn git_code_why_sources(intel: &refact_git_intel::GitIntel) -> Vec<CodeWhySource> {
    let mut sources = Vec::new();
    let mut order = 0usize;
    let significant: HashSet<usize> = intel
        .significant_commits(CODE_WHY_SIGNIFICANT_COMMITS)
        .into_iter()
        .collect();

    for (idx, commit) in intel.commit_records.iter().enumerate() {
        if merge_commit_has_body(&commit.message) {
            sources.push(CodeWhySource {
                kind: "pr".to_string(),
                text: commit.message.clone(),
                reference: short_oid(commit.oid.as_ref(), idx),
                priority: code_why_priority("pr"),
                order,
            });
            order += 1;
        }
        if !significant.contains(&idx) {
            continue;
        }
        if commit.message.trim().is_empty() {
            continue;
        }
        sources.push(CodeWhySource {
            kind: "commit".to_string(),
            text: commit_prose_text(&commit.message),
            reference: short_oid(commit.oid.as_ref(), idx),
            priority: code_why_priority("commit"),
            order,
        });
        order += 1;
    }

    sources
}

fn glob_relative_files(repo_path: &Path, pattern: &str) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    let full_pattern = repo_path.join(pattern).to_string_lossy().to_string();
    let Ok(paths) = glob::glob(&full_pattern) else {
        return out;
    };
    for path in paths.flatten() {
        if !path.is_file() {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(repo_path) {
            out.insert(relative.to_path_buf());
        }
    }
    out
}

fn code_why_file_kinds(repo_path: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut by_path: BTreeMap<PathBuf, &'static str> = BTreeMap::new();
    for pattern in [
        "docs/**/adr/**/*.md",
        "docs/adr*/*.md",
        "ADR-*.md",
        "docs/decisions/**",
    ] {
        for path in glob_relative_files(repo_path, pattern) {
            by_path.insert(path, "adr");
        }
    }
    for pattern in ["CHANGELOG*.md", "docs/CHANGELOG*.md"] {
        for path in glob_relative_files(repo_path, pattern) {
            by_path.entry(path).or_insert("changelog");
        }
    }
    by_path.into_iter().collect()
}

fn code_why_file_candidates(repo_path: &Path) -> Vec<CodeWhyFileCandidate> {
    code_why_file_kinds(repo_path)
        .into_iter()
        .enumerate()
        .map(|(index, (relative, kind))| CodeWhyFileCandidate {
            reference: path_ref(&relative),
            relative,
            kind,
            priority: code_why_priority(kind),
            order: usize::MAX / 2 + index,
        })
        .collect()
}

fn sort_code_why_source_meta(items: &mut [CodeWhySourceMeta]) {
    items.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.reference.cmp(&right.reference))
            .then_with(|| left.order.cmp(&right.order))
    });
}

fn select_code_why_file_candidates(
    candidates: Vec<CodeWhyFileCandidate>,
    git_sources: &[CodeWhySource],
    limit: usize,
) -> Vec<CodeWhyFileCandidate> {
    let mut slots = git_sources
        .iter()
        .map(|source| CodeWhySourceMeta {
            priority: source.priority,
            order: source.order,
            reference: source.reference.clone(),
            file: None,
        })
        .chain(candidates.into_iter().map(|candidate| CodeWhySourceMeta {
            priority: candidate.priority,
            order: candidate.order,
            reference: candidate.reference.clone(),
            file: Some(candidate),
        }))
        .collect::<Vec<_>>();
    sort_code_why_source_meta(&mut slots);
    slots
        .into_iter()
        .take(limit)
        .filter_map(|slot| slot.file)
        .collect()
}

async fn file_code_why_sources(
    gcx: Arc<crate::global_context::GlobalContext>,
    repo_path: &Path,
    candidates: Vec<CodeWhyFileCandidate>,
) -> Vec<CodeWhySource> {
    let mut sources = Vec::new();
    for candidate in candidates {
        let absolute = repo_path.join(&candidate.relative);
        #[cfg(test)]
        CODE_WHY_FILE_READ_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Ok(text) =
            crate::files_in_workspace::get_file_text_from_memory_or_disk(gcx.clone(), &absolute)
                .await
        else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        sources.push(CodeWhySource {
            kind: candidate.kind.to_string(),
            text,
            reference: candidate.reference,
            priority: candidate.priority,
            order: candidate.order,
        });
    }
    sources
}

#[cfg(test)]
fn code_why_selected_candidate_count_before_reads(
    repo_path: &Path,
    git_sources: &[CodeWhySource],
    limit: usize,
) -> usize {
    select_code_why_file_candidates(code_why_file_candidates(repo_path), git_sources, limit).len()
}

fn sort_code_why_sources(sources: &mut [CodeWhySource]) {
    sources.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.order.cmp(&right.order))
            .then_with(|| left.reference.cmp(&right.reference))
    });
}

fn cap_code_why_sources(mut sources: Vec<CodeWhySource>) -> Vec<CodeWhySource> {
    sort_code_why_sources(&mut sources);
    sources.truncate(CODE_WHY_SOURCE_LIMIT);
    sources
}

async fn assemble_code_why_sources(
    gcx: Arc<crate::global_context::GlobalContext>,
    repo_path: &Path,
    intel: &refact_git_intel::GitIntel,
) -> Vec<CodeWhySource> {
    let mut sources = git_code_why_sources(intel);
    let file_candidates = select_code_why_file_candidates(
        code_why_file_candidates(repo_path),
        &sources,
        CODE_WHY_SOURCE_LIMIT,
    );
    sources.extend(file_code_why_sources(gcx, repo_path, file_candidates).await);
    cap_code_why_sources(sources)
}

fn code_why_candidates(sources: &[CodeWhySource]) -> Vec<CodeWhyCandidate> {
    let mut out = Vec::new();
    for source in sources {
        let decision_source = refact_codewiki::DecisionSource {
            kind: source.kind.clone(),
            text: source.text.clone(),
        };
        for extracted in refact_codewiki::sources::extract_all(&[decision_source]) {
            out.push(CodeWhyCandidate {
                decision: refact_codewiki::Decision {
                    statement: extracted.statement,
                    evidence: extracted.evidence,
                    source_kind: extracted.source_kind,
                    status: extracted.status,
                    provenance: extracted.provenance,
                    corroboration_count: 1,
                },
                source_rank: extracted.source_rank,
                source_ref: source.reference.clone(),
            });
        }
    }
    out
}

fn better_code_why_candidate(left: &CodeWhyCandidate, right: &CodeWhyCandidate) -> bool {
    left.source_rank > right.source_rank
        || (left.source_rank == right.source_rank && left.source_ref < right.source_ref)
}

fn representative_candidate<'a>(
    decision: &refact_codewiki::Decision,
    candidates: &'a [CodeWhyCandidate],
) -> Option<&'a CodeWhyCandidate> {
    let mut best = None;
    for candidate in candidates {
        if !refact_codewiki::semantic_match::is_duplicate_decision_text(
            &decision.statement,
            &candidate.decision.statement,
        ) {
            continue;
        }
        if best
            .map(|existing| better_code_why_candidate(candidate, existing))
            .unwrap_or(true)
        {
            best = Some(candidate);
        }
    }
    best
}

fn corroborated_code_why_decisions(candidates: &[CodeWhyCandidate]) -> Vec<CodeWhyDecision> {
    let mut decisions: Vec<refact_codewiki::Decision> = candidates
        .iter()
        .map(|candidate| candidate.decision.clone())
        .collect();
    refact_codewiki::corroborate(&mut decisions);
    decisions
        .into_iter()
        .map(|decision| {
            let (source_rank, source_ref) = representative_candidate(&decision, candidates)
                .map(|candidate| (candidate.source_rank, candidate.source_ref.clone()))
                .unwrap_or_else(|| {
                    (
                        source_rank_for_kind(&decision.source_kind),
                        decision.source_kind.clone(),
                    )
                });
            CodeWhyDecision {
                decision,
                source_rank,
                source_ref,
            }
        })
        .collect()
}

fn source_rank_for_kind(kind: &str) -> u8 {
    match kind {
        "adr" => refact_codewiki::sources::SOURCE_RANK_ADR,
        "pr" => refact_codewiki::sources::SOURCE_RANK_PR,
        "commit" => refact_codewiki::sources::SOURCE_RANK_COMMIT,
        "changelog" => refact_codewiki::sources::SOURCE_RANK_CHANGELOG,
        "code_comment" => refact_codewiki::sources::SOURCE_RANK_CODE_COMMENT,
        _ => refact_codewiki::sources::SOURCE_RANK_COMMENT,
    }
}

fn verification_for_status(
    status: refact_codewiki::DecisionStatus,
) -> refact_codewiki::Verification {
    match status {
        refact_codewiki::DecisionStatus::Verified => refact_codewiki::Verification::Exact,
        refact_codewiki::DecisionStatus::Fuzzy => refact_codewiki::Verification::Fuzzy,
        refact_codewiki::DecisionStatus::Unverified => refact_codewiki::Verification::Unverified,
    }
}

fn score_code_why_decision(decision: CodeWhyDecision) -> ScoredCodeWhyDecision {
    let verification = verification_for_status(decision.decision.status);
    let confidence = refact_codewiki::gate::compute_confidence(
        decision.source_rank,
        decision.decision.corroboration_count,
        (decision.decision.provenance, verification),
    );
    ScoredCodeWhyDecision {
        item: decision,
        confidence,
        verification,
    }
}

fn sort_scored_code_why_decisions(decisions: &mut [ScoredCodeWhyDecision]) {
    decisions.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| left.item.source_ref.cmp(&right.item.source_ref))
            .then_with(|| {
                left.item
                    .decision
                    .statement
                    .cmp(&right.item.decision.statement)
            })
    });
}

fn provenance_tag(
    provenance: refact_codewiki::Provenance,
    verification: refact_codewiki::Verification,
) -> &'static str {
    match (provenance, verification) {
        (refact_codewiki::Provenance::Verbatim, _) => "verbatim",
        (refact_codewiki::Provenance::Paraphrase, refact_codewiki::Verification::Unverified) => {
            "paraphrase-unverified"
        }
        (refact_codewiki::Provenance::Paraphrase, _) => "paraphrase-verified",
    }
}

fn related_decision_graph(
    matched: &[ScoredCodeWhyDecision],
    all: &[CodeWhyDecision],
) -> Option<refact_codewiki::DecisionGraph> {
    let mut included = BTreeSet::new();
    for matched_item in matched.iter().take(5) {
        let Some(self_idx) = all.iter().position(|candidate| {
            candidate.decision.statement == matched_item.item.decision.statement
                && candidate.source_ref == matched_item.item.source_ref
        }) else {
            continue;
        };
        let mut hits: Vec<refact_codewiki::semantic_match::SearchHit> = all
            .iter()
            .enumerate()
            .map(
                |(idx, candidate)| refact_codewiki::semantic_match::SearchHit {
                    page_id: refact_codewiki::semantic_match::decision_page_id(&idx.to_string()),
                    score: refact_codewiki::evolution::text_similarity(
                        &matched_item.item.decision.statement,
                        &candidate.decision.statement,
                    ),
                },
            )
            .collect();
        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.page_id.cmp(&right.page_id))
        });
        let exclude_ids = HashSet::from([self_idx.to_string()]);
        let related = refact_codewiki::semantic_match::find_related_decisions(
            &hits,
            refact_codewiki::evolution::RELATED_TAU,
            refact_codewiki::semantic_match::DEFAULT_DEDUP_TAU,
            &exclude_ids,
            CODE_WHY_RELATED_PER_MATCH,
        );
        if related.is_empty() {
            continue;
        }
        included.insert(self_idx);
        for (id, _) in related {
            if let Ok(idx) = id.parse::<usize>() {
                included.insert(idx);
            }
        }
    }
    if included.len() < 2 {
        return None;
    }
    let decisions = included
        .into_iter()
        .filter_map(|idx| all.get(idx).map(|item| item.decision.clone()))
        .collect::<Vec<_>>();
    let graph = refact_codewiki::build_decision_graph(decisions);
    (!graph.edges.is_empty()).then_some(graph)
}

fn relation_label(relation: refact_codewiki::Relation) -> &'static str {
    match relation {
        refact_codewiki::Relation::Supersedes => "supersedes",
        refact_codewiki::Relation::Refines => "refines",
        refact_codewiki::Relation::RelatesTo => "relates",
        refact_codewiki::Relation::ConflictsWith => "conflicts",
    }
}

fn code_why_output(
    query: &str,
    sources: &[CodeWhySource],
    commits_analyzed: u32,
) -> Option<String> {
    let candidates = code_why_candidates(sources);
    let decisions = corroborated_code_why_decisions(&candidates);
    let needle = query.to_lowercase();
    let mut all_scored: Vec<ScoredCodeWhyDecision> =
        decisions.into_iter().map(score_code_why_decision).collect();
    sort_scored_code_why_decisions(&mut all_scored);
    let mut matched: Vec<ScoredCodeWhyDecision> = all_scored
        .iter()
        .filter(|scored| {
            scored
                .item
                .decision
                .statement
                .to_lowercase()
                .contains(&needle)
        })
        .cloned()
        .collect();
    matched.truncate(CODE_WHY_MATCH_LIMIT);
    if matched.is_empty() {
        return None;
    }

    let mut msg = format!(
        "Decisions matching `{query}` from {} sources ({} commits analyzed):\n",
        sources.len(),
        commits_analyzed
    );
    for scored in &matched {
        let d = &scored.item.decision;
        let tag = provenance_tag(d.provenance, scored.verification);
        msg.push_str(&format!(
            "  [{}] conf={:.2} corr={} src={}:{} — {}\n",
            tag,
            scored.confidence,
            d.corroboration_count,
            d.source_kind,
            scored.item.source_ref,
            d.statement
        ));
    }
    let all_decisions: Vec<CodeWhyDecision> = all_scored
        .iter()
        .map(|scored| scored.item.clone())
        .collect();
    if let Some(graph) = related_decision_graph(&matched, &all_decisions) {
        msg.push_str("\nRelated decision graph:\n");
        for (from, to, relation) in graph.edges.iter().take(12) {
            let Some(from_decision) = graph.decisions.get(*from) else {
                continue;
            };
            let Some(to_decision) = graph.decisions.get(*to) else {
                continue;
            };
            msg.push_str(&format!(
                "  {} {} {}\n",
                from_decision.statement,
                relation_label(*relation),
                to_decision.statement
            ));
        }
    }
    Some(msg)
}

#[async_trait]
impl Tool for ToolCodeWhy {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let query = string_arg(args, "query")?;
        let roots = codegraph_project_roots(ccx.clone())
            .await?
            .ok_or_else(|| "no project directory available".to_string())?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let intel = cached_mine_history(&roots.git_root, CODE_WHY_HISTORY_COMMITS)?;
        let sources = assemble_code_why_sources(gcx, &roots.git_root, &intel).await;
        let Some(msg) = code_why_output(&query, &sources, intel.commits_analyzed) else {
            return Ok((
                false,
                tool_message(
                    tool_call_id,
                    format!(
                        "No matching decision records found in commit prose, ADRs, changelogs, or merge PRs for `{query}`."
                    ),
                ),
            ));
        };
        Ok((false, tool_message(tool_call_id, msg)))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "code_why".to_string(),
            display_name: "Code Why".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Explain why the code is the way it is: mines significant commit prose, merge PR bodies, ADRs, and changelogs for query-matching decisions, then reports source refs, confidence, corroboration, provenance tags, and related-decision links.".to_string(),
            input_schema: json_schema_from_params(
                &[("query", "string", "Topic or keyword to find decisions about.")],
                &["query"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

pub struct ToolCodeDuplication {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolCodeDuplication {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.app.gcx.clone();
        let analysis = cached_cross_file_clones(gcx.clone()).await?;
        if analysis.clones.is_empty() {
            return Ok((
                false,
                tool_message(tool_call_id, "No cross-file clones detected.".to_string()),
            ));
        }
        let dup_pct = analysis.duplication_pct * 100.0;
        let roots = codegraph_project_roots(ccx.clone()).await?;
        let project_root = roots.as_ref().map(|roots| roots.indexed_root.as_path());
        let intel = roots
            .as_ref()
            .map(|roots| roots.git_root.as_path())
            .and_then(|dir| cached_mine_history(dir, 1000).ok());
        let co_change =
            |a: &str, b: &str| -> u32 { co_change_count(intel.as_ref(), a, b, project_root) };
        let mut msg = format!(
            "Cross-file duplication: {:.1}% of tokens are in cross-file clones ({} pairs).\n",
            dup_pct,
            analysis.clones.len()
        );
        for c in analysis.clones.iter().take(15) {
            msg.push_str(&format!(
                "  {} tokens: {}:{} <-> {}:{}\n",
                c.token_len, c.file_a, c.line_a, c.file_b, c.line_b
            ));
        }
        let mut per_file: HashMap<
            String,
            Vec<&refact_codehealth::duplication::CrossFileClonePair>,
        > = HashMap::new();
        for c in &analysis.clones {
            per_file.entry(c.file_a.clone()).or_default().push(c);
            per_file.entry(c.file_b.clone()).or_default().push(c);
        }
        let mut dry_lines = Vec::new();
        let mut smell_lines = Vec::new();
        for (path, file_clones) in &per_file {
            let dry_clones: Vec<refact_codehealth::dry::DryClonePair> = file_clones
                .iter()
                .map(|c| refact_codehealth::dry::DryClonePair {
                    file_a: c.file_a.clone(),
                    a_start_line: c.a_start_line,
                    a_line_count: c.a_end_line.saturating_sub(c.a_start_line) + 1,
                    file_b: c.file_b.clone(),
                    b_start_line: c.b_start_line,
                    b_line_count: c.b_end_line.saturating_sub(c.b_start_line) + 1,
                    co_change_count: co_change(&c.file_a, &c.file_b),
                })
                .collect();
            let file_dup_tokens = analysis
                .duplicated_tokens_by_path
                .get(path)
                .copied()
                .unwrap_or(0);
            let file_total = analysis
                .tokens_by_path
                .get(path)
                .copied()
                .unwrap_or(0)
                .max(1);
            let file_dup_pct = (file_dup_tokens as f64 / file_total as f64).min(1.0);
            let dry_input = refact_codehealth::dry::DryInput {
                file_path: path.clone(),
                duplication_pct: file_dup_pct,
                clones: dry_clones,
            };
            for f in refact_codehealth::dry::dry_violation(&dry_input) {
                dry_lines.push(format!(
                    "  {} [{:?}] {} — {}",
                    f.biomarker, f.severity, path, f.detail
                ));
            }
            if refact_git_intel::paths::is_test_path(path) {
                if let Some((lang, text)) = analysis.text_by_path.get(path) {
                    let assertion_blocks =
                        refact_codehealth::assertions::assertion_blocks(lang, text);
                    let ts_clones: Vec<refact_codehealth::test_smells::ClonePair> = file_clones
                        .iter()
                        .map(|c| refact_codehealth::test_smells::ClonePair {
                            file_a: c.file_a.clone(),
                            a_start_line: c.a_start_line,
                            a_end_line: c.a_end_line,
                            file_b: c.file_b.clone(),
                            b_start_line: c.b_start_line,
                            b_end_line: c.b_end_line,
                        })
                        .collect();
                    let ts_input = refact_codehealth::test_smells::TestSmellInput {
                        file_path: path.clone(),
                        is_test_file: true,
                        assertion_blocks,
                        clones: ts_clones,
                    };
                    for f in refact_codehealth::test_smells::test_smell_biomarkers(&ts_input) {
                        smell_lines.push(format!(
                            "  {} [{:?}] {} — {}",
                            f.biomarker, f.severity, path, f.detail
                        ));
                    }
                }
            }
        }
        dry_lines.sort();
        dry_lines.dedup();
        smell_lines.sort();
        smell_lines.dedup();
        if !dry_lines.is_empty() {
            msg.push_str("\nDRY violations:\n");
            for l in dry_lines.iter().take(15) {
                msg.push_str(l);
                msg.push('\n');
            }
        }
        if !smell_lines.is_empty() {
            msg.push_str("\nTest smells:\n");
            for l in smell_lines.iter().take(15) {
                msg.push_str(l);
                msg.push('\n');
            }
        }
        Ok((false, tool_message(tool_call_id, msg)))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "code_duplication".to_string(),
            display_name: "Code Duplication".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Project-wide cross-file duplication from a graph-generation cache: token clone pairs and duplication percentage, git co-change counts joined to clone paths, co-change-weighted DRY findings, and assertion/clone test smells.".to_string(),
            input_schema: json_schema_from_params(&[], &[]),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

pub struct ToolSecurityScan {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolSecurityScan {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let requested_file_path = string_arg(args, "file_path")?;
        let resolved_file = resolve_codegraph_file(ccx.clone(), &requested_file_path).await?;
        let file_path = resolved_file.indexed_path;
        let gcx = ccx.lock().await.app.gcx.clone();
        let text = crate::files_in_workspace::get_file_text_from_memory_or_disk(
            gcx.clone(),
            &resolved_file.read_path,
        )
        .await?
        .to_string();
        let service = gcx
            .codegraph
            .lock()
            .await
            .clone()
            .ok_or_else(|| "codegraph is not available".to_string())?;
        let lang = refact_codegraph::lang_from_path(&file_path);
        let findings = service.security_scan(&file_path, lang, &text).await?;
        if findings.is_empty() {
            return Ok((
                false,
                tool_message(
                    tool_call_id,
                    format!(
                        "Security scan for `{requested_file_path}` found no findings (lang: {lang})."
                    ),
                ),
            ));
        }

        let mut counts: HashMap<&'static str, usize> = HashMap::new();
        for finding in &findings {
            *counts.entry(severity_label(finding.severity)).or_insert(0) += 1;
        }
        let mut msg = format!(
            "Security scan for `{}` found {} findings (lang: {}).\n",
            requested_file_path,
            findings.len(),
            lang
        );
        msg.push_str(&format!(
            "Severity counts: Critical={} High={} Medium={} Low={}\n\n",
            counts.get("Critical").copied().unwrap_or(0),
            counts.get("High").copied().unwrap_or(0),
            counts.get("Medium").copied().unwrap_or(0),
            counts.get("Low").copied().unwrap_or(0)
        ));
        for finding in findings.iter().take(50) {
            msg.push_str(&format!(
                "  {}:{} [{:?}] {} — {}\n",
                requested_file_path, finding.line, finding.severity, finding.rule, finding.snippet
            ));
        }
        if findings.len() > 50 {
            msg.push_str(&format!(
                "  ... {} more findings omitted\n",
                findings.len() - 50
            ));
        }
        Ok((false, tool_message(tool_call_id, msg)))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "security_scan".to_string(),
            display_name: "Security Scan".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Security scan for one file using CodeGraph security heuristics: deduped findings for hardcoded secrets, dynamic SQL and command execution, dangerous eval/deserialization, TLS verification disabled, weak crypto, and insecure randomness.".to_string(),
            input_schema: json_schema_from_params(
                &[(
                    "file_path",
                    "string",
                    "File to scan; accepts the same workspace-relative, absolute current-project, and active-worktree path forms as cat.",
                )],
                &["file_path"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

pub struct ToolPrBlast {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolPrBlast {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let changed_files = string_list_arg(args, "changed_files")?;
        if changed_files.is_empty() {
            return Err("argument `changed_files` must not be empty".to_string());
        }
        let max_depth = optional_usize_arg(args, "max_depth", 3)?.clamp(1, 10);
        let mut resolved_changed_files = Vec::with_capacity(changed_files.len());
        for path in &changed_files {
            resolved_changed_files.push(resolve_codegraph_filter_path(ccx.clone(), path).await?);
        }
        let gcx = ccx.lock().await.app.gcx.clone();
        let service = gcx
            .codegraph
            .lock()
            .await
            .clone()
            .ok_or_else(|| "codegraph is not available".to_string())?;
        let readiness = service.index_readiness().await?;
        let index_state = pr_blast_index_state(&readiness);
        let warning = pr_blast_partial_warning(&index_state);
        let report = service.pr_blast(&resolved_changed_files, max_depth).await?;
        let roots = codegraph_project_roots(ccx.clone()).await?;
        let intel = roots
            .as_ref()
            .map(|roots| roots.git_root.as_path())
            .and_then(|dir| cached_mine_history(dir, 1000).ok());
        let reviewers = pr_blast_suggested_reviewers(
            &report,
            roots.as_ref().map(|roots| roots.indexed_root.as_path()),
            intel.as_ref(),
        );

        let mut msg = String::new();
        if let Some(warning) = warning {
            msg.push_str(&warning);
            msg.push('\n');
        }
        msg.push_str(&format!(
            "PR blast radius (max depth {}) for {} changed files:\n",
            max_depth,
            report.changed_files.len()
        ));
        msg.push_str(&format!(
            "Index state: queued={} cross_file_edges={} cross_file_ready={} partial={}\n",
            index_state.queued,
            index_state.cross_file_edges,
            index_state.cross_file_ready,
            !index_state.cross_file_ready
        ));
        for path in &report.changed_files {
            msg.push_str(&format!("  changed: {}\n", path));
        }
        msg.push_str(&format!(
            "\nImpacted files: {}\nRisk score: {:.2}\n",
            report.impacted_file_count, report.risk_score
        ));
        if report.directly_impacted.is_empty() && report.transitively_impacted.is_empty() {
            msg.push_str("\nNo reverse dependencies found for these files.\n");
        } else {
            if !report.directly_impacted.is_empty() {
                msg.push_str(&format!(
                    "\nDirectly impacted symbols ({}):\n",
                    report.directly_impacted.len()
                ));
                for impact in report.directly_impacted.iter().take(30) {
                    push_blast_impact(&mut msg, impact);
                }
                if report.directly_impacted.len() > 30 {
                    msg.push_str(&format!(
                        "  ... {} more direct impacts omitted\n",
                        report.directly_impacted.len() - 30
                    ));
                }
            }
            if !report.transitively_impacted.is_empty() {
                msg.push_str(&format!(
                    "\nTransitively impacted symbols ({}):\n",
                    report.transitively_impacted.len()
                ));
                for impact in report.transitively_impacted.iter().take(30) {
                    push_blast_impact(&mut msg, impact);
                }
                if report.transitively_impacted.len() > 30 {
                    msg.push_str(&format!(
                        "  ... {} more transitive impacts omitted\n",
                        report.transitively_impacted.len() - 30
                    ));
                }
            }
        }
        if !reviewers.is_empty() {
            msg.push_str("\nSuggested reviewers (git ownership):\n");
            for reviewer in reviewers {
                msg.push_str(&format!(
                    "  {} (score {:.2})\n",
                    reviewer.author, reviewer.score
                ));
            }
        }
        Ok((false, tool_message(tool_call_id, msg)))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "pr_blast".to_string(),
            display_name: "PR Blast Radius".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "PR blast-radius analysis for changed files: resolves indexed paths, walks reverse CodeGraph dependencies to max depth, reports direct/transitive impacted symbols, structural vs behavioral impact kind, impacted file count, risk score, git-ownership reviewer suggestions excluding bot authors, and readiness/partial-index warnings.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "changed_files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Changed files to analyze; each accepts the same workspace-relative, absolute current-project, active-worktree, or basename forms as cat. Deleted paths are allowed."
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum reverse dependency depth to walk. Defaults to 3, max 10."
                    }
                },
                "required": ["changed_files"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

fn severity_label(severity: refact_codegraph::security_scan::Severity) -> &'static str {
    match severity {
        refact_codegraph::security_scan::Severity::Low => "Low",
        refact_codegraph::security_scan::Severity::Medium => "Medium",
        refact_codegraph::security_scan::Severity::High => "High",
        refact_codegraph::security_scan::Severity::Critical => "Critical",
    }
}

fn push_blast_impact(msg: &mut String, impact: &refact_codegraph::pr_blast::BlastImpact) {
    msg.push_str(&format!(
        "  d{} {} @ {} via {} ({})\n",
        impact.distance,
        impact.symbol,
        impact.path,
        impact.via,
        blast_impact_kind_label(impact.kind)
    ));
}

fn blast_impact_kind_label(kind: refact_codegraph::pr_blast::ImpactKind) -> &'static str {
    match kind {
        refact_codegraph::pr_blast::ImpactKind::Behavioral => "behavioral",
        refact_codegraph::pr_blast::ImpactKind::Structural => "structural",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeMapNode {
    id: i64,
    kind: String,
    name: String,
    path: String,
    visibility: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct CodeMapFile {
    parsed: refact_codewiki::ParsedFile,
    text: String,
    is_hotspot: bool,
    kg_bonus: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeMapLink {
    target_path: String,
    labels: Vec<String>,
    count: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct CodeMapRenderedPage {
    page: refact_codewiki::AllocatedPage,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodeMapFormat {
    Markdown,
    ClaudeMd,
}

impl CodeMapFormat {
    fn parse(value: Option<&Value>) -> Result<Self, String> {
        match value {
            None | Some(Value::Null) => Ok(Self::Markdown),
            Some(Value::String(value)) if value == "markdown" => Ok(Self::Markdown),
            Some(Value::String(value)) if value == "claude_md" => Ok(Self::ClaudeMd),
            Some(Value::String(value)) => Err(format!(
                "argument `format` must be `markdown` or `claude_md`, got `{value}`"
            )),
            Some(value) => Err(format!(
                "argument `format` must be `markdown` or `claude_md`: {value:?}"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeMapArgs {
    query: Option<String>,
    format: CodeMapFormat,
    budget_tokens: usize,
}

fn code_map_args(args: &HashMap<String, Value>) -> Result<CodeMapArgs, String> {
    Ok(CodeMapArgs {
        query: optional_string_arg(args, "query")?,
        format: CodeMapFormat::parse(args.get("format"))?,
        budget_tokens: optional_usize_arg(args, "budget", CODE_MAP_DEFAULT_BUDGET_TOKENS)?.max(1),
    })
}

fn code_map_node_kind(kind: &str) -> String {
    match kind {
        "typealias" => "type_alias".to_string(),
        "route" => "route".to_string(),
        value => value.to_string(),
    }
}

fn code_map_node_visibility(data: Option<&str>) -> Option<String> {
    let data = data?;
    let value = serde_json::from_str::<Value>(data).ok()?;
    let visibility = value.get("visibility")?.as_str()?.trim();
    (!visibility.is_empty()).then(|| visibility.to_string())
}

fn code_map_nodes_from_records(
    records: Vec<(i64, String, String, String, Option<String>)>,
) -> Vec<CodeMapNode> {
    let mut nodes = records
        .into_iter()
        .map(|(id, kind, name, path, data)| CodeMapNode {
            id,
            kind: code_map_node_kind(&kind),
            name,
            path,
            visibility: code_map_node_visibility(data.as_deref()),
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    nodes
}

fn code_map_symbols_by_path(
    nodes: &[CodeMapNode],
) -> HashMap<String, Vec<refact_codewiki::SymbolInfo>> {
    let mut symbols = HashMap::<String, Vec<refact_codewiki::SymbolInfo>>::new();
    for node in nodes {
        if node.kind == "route" || node.kind == "unknown" {
            continue;
        }
        symbols
            .entry(node.path.clone())
            .or_default()
            .push(refact_codewiki::SymbolInfo {
                kind: node.kind.clone(),
                visibility: node.visibility.clone().unwrap_or_default(),
            });
    }
    for values in symbols.values_mut() {
        values.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.visibility.cmp(&right.visibility))
        });
    }
    symbols
}

fn normalize_file_scores(values: &HashMap<String, f64>) -> HashMap<String, f64> {
    let max = values
        .values()
        .copied()
        .filter(|value| value.is_finite())
        .fold(0.0_f64, f64::max);
    values
        .iter()
        .map(|(path, score)| {
            let normalized = if max > 0.0 && score.is_finite() {
                (score / max).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (path.clone(), normalized)
        })
        .collect()
}

fn code_map_is_hotspot(
    path: &str,
    intel: Option<&refact_git_intel::GitIntel>,
    repo_root: Option<&Path>,
) -> bool {
    let Some(intel) = intel else {
        return false;
    };
    let key = match repo_root {
        Some(root) => {
            refact_git_intel::paths::repo_relative_or_basename(path, &root.to_string_lossy())
        }
        None => refact_git_intel::paths::normalize_separators(path),
    };
    intel.churn_percentile(&key) >= 0.9
}

fn code_map_infra_path(path: &str) -> bool {
    let normalized = refact_git_intel::paths::normalize_separators(path).to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    matches!(
        name,
        "dockerfile"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "makefile"
            | "gnumakefile"
            | "package.json"
            | "package-lock.json"
            | "cargo.toml"
            | "cargo.lock"
            | "pyproject.toml"
            | "requirements.txt"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "pom.xml"
    ) || normalized.starts_with(".github/workflows/")
        || normalized.contains("/.github/workflows/")
}

fn top_level_module(path: &str) -> Option<String> {
    path.split('/')
        .find(|segment| !segment.is_empty())
        .map(str::to_string)
}

fn second_level_module(path: &str) -> Option<String> {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .take(2)
        .collect::<Vec<_>>();
    (segments.len() == 2).then(|| segments.join("/"))
}

fn module_selections(files: &[CodeMapFile]) -> Vec<refact_codewiki::ModuleSelection> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in files {
        let path = file.parsed.file_info.path.as_str();
        if let Some(module) = top_level_module(path) {
            grouped.entry(module).or_default().push(path.to_string());
        }
        if let Some(module) = second_level_module(path) {
            grouped.entry(module).or_default().push(path.to_string());
        }
    }
    grouped
        .into_iter()
        .filter_map(|(id, mut paths)| {
            paths.sort();
            paths.dedup();
            if paths.len() < CODE_MAP_MIN_MODULE_FILES {
                return None;
            }
            Some(refact_codewiki::ModuleSelection {
                id,
                size: paths.len() as u64,
                cohesion: 0.5,
                paths,
            })
        })
        .collect()
}

fn scc_path_groups(
    nodes: &[CodeMapNode],
    edges: &[refact_codegraph::analytics::GraphEdge],
) -> Vec<Vec<String>> {
    let mut graph = DiGraph::<i64, ()>::new();
    let mut index_by_id = HashMap::<i64, NodeIndex>::new();
    for node in nodes {
        index_by_id
            .entry(node.id)
            .or_insert_with(|| graph.add_node(node.id));
    }
    for (src, dst, kind) in edges {
        if !CODE_MAP_LINK_EDGE_KINDS.contains(&kind.as_str()) {
            continue;
        }
        let (Some(src_idx), Some(dst_idx)) = (index_by_id.get(src), index_by_id.get(dst)) else {
            continue;
        };
        graph.add_edge(*src_idx, *dst_idx, ());
    }
    let path_by_id: HashMap<i64, &str> = nodes
        .iter()
        .map(|node| (node.id, node.path.as_str()))
        .collect();
    let mut out = Vec::new();
    for component in tarjan_scc(&graph) {
        if component.len() <= 1 {
            continue;
        }
        let mut paths = component
            .into_iter()
            .filter_map(|idx| path_by_id.get(&graph[idx]).map(|path| (*path).to_string()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if paths.len() <= 1 {
            continue;
        }
        paths.sort();
        out.push(paths);
    }
    out.sort();
    out
}

fn build_code_map_files(
    files_text: &[(String, String)],
    nodes: &[CodeMapNode],
    centrality: &refact_codegraph::analytics::FileCentrality,
    intel: Option<&refact_git_intel::GitIntel>,
    repo_root: Option<&Path>,
) -> Vec<CodeMapFile> {
    let pr: HashMap<String, f64> = centrality.top_pagerank.iter().cloned().collect();
    let normalized_pr = normalize_file_scores(&pr);
    let symbols_by_path = code_map_symbols_by_path(nodes);
    let mut out = files_text
        .iter()
        .map(|(path, text)| {
            let parsed = refact_codewiki::ParsedFile {
                file_info: refact_codewiki::FileInfo {
                    path: path.clone(),
                    is_entry_point: refact_codewiki::entry_points::is_conventional_entry(path),
                    is_test: refact_git_intel::paths::is_test_path(path),
                    size_bytes: text.len() as u64,
                },
                symbols: symbols_by_path.get(path).cloned().unwrap_or_default(),
            };
            CodeMapFile {
                parsed,
                text: text.clone(),
                is_hotspot: code_map_is_hotspot(path, intel, repo_root),
                kg_bonus: normalized_pr.get(path).copied().unwrap_or(0.0),
            }
        })
        .collect::<Vec<_>>();
    out.sort_by(|left, right| left.parsed.file_info.path.cmp(&right.parsed.file_info.path));
    out
}

fn code_map_selection_input(
    files: Vec<CodeMapFile>,
    nodes: &[CodeMapNode],
    edges: &[refact_codegraph::analytics::GraphEdge],
    centrality: &refact_codegraph::analytics::FileCentrality,
) -> refact_codewiki::SelectionInput {
    let pr: HashMap<String, f64> = centrality.top_pagerank.iter().cloned().collect();
    let bt: HashMap<String, f64> = centrality.top_betweenness.iter().cloned().collect();
    let api_contract_paths = files
        .iter()
        .filter_map(|file| {
            let path = file.parsed.file_info.path.as_str();
            let lang = refact_codegraph::lang_from_path(path);
            let parsed = refact_codewiki::parsed_file::build_parsed_file(lang, &file.text);
            refact_codewiki::api_contract::detect_api_contract(&parsed).then(|| path.to_string())
        })
        .collect();
    let infra_paths = files
        .iter()
        .filter_map(|file| {
            code_map_infra_path(&file.parsed.file_info.path)
                .then(|| file.parsed.file_info.path.clone())
        })
        .collect();
    let modules = module_selections(&files);
    let selections = files
        .into_iter()
        .map(|file| {
            let path = file.parsed.file_info.path.clone();
            refact_codewiki::FileSelection {
                parsed: file.parsed,
                pagerank: pr.get(&path).copied().unwrap_or(0.0),
                betweenness: bt.get(&path).copied().unwrap_or(0.0),
                is_hotspot: file.is_hotspot,
                kg_bonus: file.kg_bonus,
            }
        })
        .collect();
    refact_codewiki::SelectionInput {
        files: selections,
        modules,
        sccs: scc_path_groups(nodes, edges),
        api_contract_paths,
        infra_paths,
        min_module_size: CODE_MAP_MIN_MODULE_SIZE,
    }
}

fn page_title(page: &refact_codewiki::PageCandidate) -> String {
    page.id
        .split_once(':')
        .map(|(_, title)| title)
        .unwrap_or(page.id.as_str())
        .to_string()
}

fn page_kind_label(kind: refact_codewiki::PageKind) -> &'static str {
    match kind {
        refact_codewiki::PageKind::File => "file",
        refact_codewiki::PageKind::Module => "module",
        refact_codewiki::PageKind::Scc => "scc",
        refact_codewiki::PageKind::ApiContract => "api-contract",
        refact_codewiki::PageKind::Infra => "infra",
    }
}

fn page_paths(page: &refact_codewiki::PageCandidate) -> BTreeSet<String> {
    page.paths.iter().cloned().collect()
}

fn code_map_links_for_page(
    page: &refact_codewiki::PageCandidate,
    nodes: &[CodeMapNode],
    edges: &[refact_codegraph::analytics::GraphEdge],
) -> Vec<CodeMapLink> {
    let page_paths = page_paths(page);
    let path_by_id: HashMap<i64, &str> = nodes
        .iter()
        .map(|node| (node.id, node.path.as_str()))
        .collect();
    let mut links = BTreeMap::<String, BTreeMap<String, usize>>::new();
    for (src, dst, kind) in edges {
        if !CODE_MAP_LINK_EDGE_KINDS.contains(&kind.as_str()) {
            continue;
        }
        let (Some(src_path), Some(dst_path)) = (path_by_id.get(src), path_by_id.get(dst)) else {
            continue;
        };
        if src_path == dst_path {
            continue;
        }
        let target = if page_paths.contains(*src_path) && !page_paths.contains(*dst_path) {
            Some(*dst_path)
        } else if page_paths.contains(*dst_path) && !page_paths.contains(*src_path) {
            Some(*src_path)
        } else {
            None
        };
        if let Some(target) = target {
            *links
                .entry(target.to_string())
                .or_default()
                .entry(kind.clone())
                .or_default() += 1;
        }
    }
    let mut out = links
        .into_iter()
        .map(|(target_path, labels)| {
            let count = labels.values().sum();
            CodeMapLink {
                target_path,
                labels: labels.into_keys().collect(),
                count,
            }
        })
        .collect::<Vec<_>>();
    out.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.target_path.cmp(&right.target_path))
    });
    out.truncate(CODE_MAP_LINK_LIMIT);
    out
}

fn symbol_kind_counts(
    page: &refact_codewiki::PageCandidate,
    nodes: &[CodeMapNode],
) -> Vec<(String, usize)> {
    let page_paths = page_paths(page);
    let mut counts = BTreeMap::<String, usize>::new();
    for node in nodes {
        if !page_paths.contains(&node.path) || node.kind == "route" || node.kind == "unknown" {
            continue;
        }
        *counts.entry(node.kind.clone()).or_default() += 1;
    }
    counts.into_iter().collect()
}

fn symbol_visibility_counts(
    page: &refact_codewiki::PageCandidate,
    nodes: &[CodeMapNode],
) -> Vec<(String, usize)> {
    let page_paths = page_paths(page);
    let mut counts = BTreeMap::<String, usize>::new();
    for node in nodes {
        if !page_paths.contains(&node.path) {
            continue;
        }
        if let Some(visibility) = &node.visibility {
            *counts.entry(visibility.clone()).or_default() += 1;
        }
    }
    counts.into_iter().collect()
}

fn trim_to_token_allowance(mut content: String, token_allowance: usize) -> String {
    if refact_codewiki::token_budget::estimate_tokens(&content) <= token_allowance {
        return content;
    }
    if token_allowance == 0 {
        return String::new();
    }
    let suffix = "...";
    let suffix_tokens = refact_codewiki::token_budget::estimate_tokens(suffix).max(1);
    if token_allowance <= suffix_tokens {
        return suffix.to_string();
    }
    let max_chars = token_allowance
        .saturating_sub(suffix_tokens)
        .saturating_mul(4);
    if content.chars().count() > max_chars {
        content = content.chars().take(max_chars).collect();
    }
    let target_tokens = token_allowance.saturating_sub(suffix_tokens);
    while refact_codewiki::token_budget::estimate_tokens(&content) > target_tokens
        && !content.is_empty()
    {
        content.pop();
    }
    content.push_str(suffix);
    content
}

fn page_signal_labels(
    page: &refact_codewiki::PageCandidate,
    input: &refact_codewiki::SelectionInput,
) -> Vec<String> {
    let page_paths = page_paths(page);
    let mut labels = BTreeSet::new();
    for file in &input.files {
        if !page_paths.contains(&file.parsed.file_info.path) {
            continue;
        }
        if file.parsed.file_info.is_entry_point {
            labels.insert("entry_point".to_string());
        }
        if file.parsed.file_info.is_test {
            labels.insert("test".to_string());
        }
        if file.is_hotspot {
            labels.insert("hotspot".to_string());
        }
        if file.kg_bonus > 0.0 {
            labels.insert(format!("kg_prior={:.2}", file.kg_bonus));
        }
    }
    labels.into_iter().collect()
}

fn render_code_map_page_markdown(
    allocation: &refact_codewiki::AllocatedPage,
    input: &refact_codewiki::SelectionInput,
    nodes: &[CodeMapNode],
    edges: &[refact_codegraph::analytics::GraphEdge],
) -> String {
    let page = &allocation.page;
    let mut content = String::new();
    content.push_str(&format!(
        "### {} — {}\n",
        page_title(page),
        page_kind_label(page.kind)
    ));
    content.push_str(&format!("Score: {:.3}\n", page.score));
    content.push_str(&format!("Paths: {}\n", page.paths.join(", ")));
    let signals = page_signal_labels(page, input);
    if !signals.is_empty() {
        content.push_str(&format!("Signals: {}\n", signals.join(", ")));
    }
    let symbol_counts = symbol_kind_counts(page, nodes);
    if !symbol_counts.is_empty() {
        content.push_str("Symbols:");
        for (kind, count) in symbol_counts {
            content.push_str(&format!(" {}={}", kind, count));
        }
        content.push('\n');
    }
    let visibility_counts = symbol_visibility_counts(page, nodes);
    if !visibility_counts.is_empty() {
        content.push_str("Visibility:");
        for (visibility, count) in visibility_counts {
            content.push_str(&format!(" {}={}", visibility, count));
        }
        content.push('\n');
    }
    let links = code_map_links_for_page(page, nodes, edges);
    if !links.is_empty() {
        content.push_str("Links:\n");
        for link in links {
            content.push_str(&format!(
                "- {} ({}, {} edges)\n",
                link.target_path,
                link.labels.join("/"),
                link.count
            ));
        }
    }
    trim_to_token_allowance(content, allocation.token_allowance)
}

fn code_map_backlink_hubs(
    rendered_pages: &[CodeMapRenderedPage],
    nodes: &[CodeMapNode],
    edges: &[refact_codegraph::analytics::GraphEdge],
) -> Vec<(String, usize)> {
    let mut counts = BTreeMap::<String, usize>::new();
    for rendered in rendered_pages {
        for link in code_map_links_for_page(&rendered.page.page, nodes, edges) {
            *counts.entry(link.target_path).or_default() += link.count;
        }
    }
    let mut hubs = counts.into_iter().collect::<Vec<_>>();
    hubs.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    hubs
}

fn code_map_page_prior(pages: &[refact_codewiki::PageCandidate]) -> HashMap<String, f64> {
    let max_score = pages
        .iter()
        .map(|page| page.score)
        .filter(|score| score.is_finite())
        .fold(0.0_f64, f64::max);
    pages
        .iter()
        .map(|page| {
            let prior = if max_score > 0.0 && page.score.is_finite() {
                page.score / max_score
            } else {
                0.0
            };
            (page.id.clone(), prior.clamp(0.0, 1.0))
        })
        .collect()
}

fn choose_code_map_pages(
    mut pages: Vec<refact_codewiki::PageCandidate>,
    query: Option<&str>,
) -> Vec<refact_codewiki::PageCandidate> {
    if let Some(query) = query {
        let prior = code_map_page_prior(&pages);
        let mut hits = refact_codewiki::search_hybrid(query, &pages, &prior);
        if hits.is_empty() {
            pages.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| left.id.cmp(&right.id))
            });
            pages.truncate(CODE_MAP_QUERY_LIMIT);
            return pages;
        }
        return hits
            .drain(..)
            .into_iter()
            .take(CODE_MAP_QUERY_LIMIT)
            .map(|hit| hit.page.clone())
            .collect();
    }
    pages.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    pages.truncate(CODE_MAP_MAX_PAGES_WITHOUT_QUERY);
    pages
}

fn render_code_map_markdown(
    files_count: usize,
    index_state: Option<&PrBlastIndexState>,
    rendered_pages: &[CodeMapRenderedPage],
    nodes: &[CodeMapNode],
    edges: &[refact_codegraph::analytics::GraphEdge],
    query: Option<&str>,
) -> String {
    let link_count: usize = rendered_pages
        .iter()
        .map(|page| code_map_links_for_page(&page.page.page, nodes, edges).len())
        .sum();
    let mut msg = String::new();
    if let Some(state) = index_state {
        if let Some(warning) = pr_blast_partial_warning(state) {
            msg.push_str(&warning);
            msg.push('\n');
        }
    }
    msg.push_str(&format!(
        "Code map: {} indexed files, {} documentation-worthy pages, {} cross-file links.",
        files_count,
        rendered_pages.len(),
        link_count
    ));
    if let Some(query) = query {
        msg.push_str(&format!(" Query: `{query}`."));
    }
    msg.push_str("\n\nMost documentation-worthy files (selection score):\n");
    for rendered in rendered_pages
        .iter()
        .filter(|rendered| rendered.page.page.kind == refact_codewiki::PageKind::File)
        .take(12)
    {
        let path = rendered
            .page
            .page
            .paths
            .first()
            .map(String::as_str)
            .unwrap_or(rendered.page.page.id.as_str());
        msg.push_str(&format!("  {:.2}  {}\n", rendered.page.page.score, path));
    }
    let hubs = code_map_backlink_hubs(rendered_pages, nodes, edges);
    let hub_lines = hubs
        .iter()
        .filter(|(_, count)| *count > 0)
        .take(12)
        .collect::<Vec<_>>();
    if !hub_lines.is_empty() {
        msg.push_str("\nMost-referenced files (backlink hubs):\n");
        for (path, count) in hub_lines {
            msg.push_str(&format!("  {}x  {}\n", count, path));
        }
    }
    msg.push_str("\nPages:\n");
    for rendered in rendered_pages {
        msg.push('\n');
        msg.push_str(&rendered.content);
    }
    msg
}

fn render_code_map_claude_md(
    rendered_pages: &[CodeMapRenderedPage],
    index_state: Option<&PrBlastIndexState>,
) -> String {
    let pages = rendered_pages
        .iter()
        .map(|rendered| rendered.page.page.clone())
        .collect::<Vec<_>>();
    let mut out = String::new();
    if let Some(state) = index_state {
        if let Some(warning) = pr_blast_partial_warning(state) {
            out.push_str(&warning);
            out.push('\n');
        }
    }
    out.push_str(&refact_codewiki::render_claude_md(&pages));
    out
}

fn assemble_code_map_output(
    files_text: Vec<(String, String)>,
    node_records: Vec<(i64, String, String, String, Option<String>)>,
    edges: Vec<refact_codegraph::analytics::GraphEdge>,
    centrality: refact_codegraph::analytics::FileCentrality,
    index_state: Option<PrBlastIndexState>,
    intel: Option<&refact_git_intel::GitIntel>,
    repo_root: Option<&Path>,
    args: &CodeMapArgs,
) -> String {
    if files_text.is_empty() {
        let mut msg = String::new();
        if let Some(state) = index_state.as_ref() {
            if let Some(warning) = pr_blast_partial_warning(state) {
                msg.push_str(&warning);
                msg.push('\n');
            }
        }
        msg.push_str("Code graph has no indexed files (index empty/building).");
        return msg;
    }
    let nodes = code_map_nodes_from_records(node_records);
    let files = build_code_map_files(&files_text, &nodes, &centrality, intel, repo_root);
    let input = code_map_selection_input(files, &nodes, &edges, &centrality);
    let selected_pages =
        choose_code_map_pages(refact_codewiki::select_pages(&input), args.query.as_deref());
    let allocations = refact_codewiki::allocate(&selected_pages, args.budget_tokens);
    let rendered_pages = allocations
        .into_iter()
        .map(|page| {
            let content = render_code_map_page_markdown(&page, &input, &nodes, &edges);
            CodeMapRenderedPage { page, content }
        })
        .collect::<Vec<_>>();
    let output = match args.format {
        CodeMapFormat::Markdown => render_code_map_markdown(
            files_text.len(),
            index_state.as_ref(),
            &rendered_pages,
            &nodes,
            &edges,
            args.query.as_deref(),
        ),
        CodeMapFormat::ClaudeMd => render_code_map_claude_md(&rendered_pages, index_state.as_ref()),
    };
    trim_to_token_allowance(output, args.budget_tokens)
}

pub struct ToolCodeMap {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolCodeMap {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let args = code_map_args(args)?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let service = gcx
            .codegraph
            .lock()
            .await
            .clone()
            .ok_or_else(|| "codegraph is not available".to_string())?;
        let files_text = service.all_files_with_text().await?;
        let cached = service.cached_graph_analytics().await?;
        let node_records = service.graph_node_records().await?;
        let readiness = service.index_readiness().await?;
        let roots = codegraph_project_roots(ccx.clone()).await?;
        let intel = roots
            .as_ref()
            .map(|roots| roots.git_root.as_path())
            .and_then(|dir| cached_mine_history(dir, 1000).ok());
        let msg = assemble_code_map_output(
            files_text,
            node_records,
            cached.data.edges,
            cached.analytics.file_centrality.truncated(5000),
            Some(pr_blast_index_state(&readiness)),
            intel.as_ref(),
            roots.as_ref().map(|roots| roots.indexed_root.as_path()),
            &args,
        );
        Ok((false, tool_message(tool_call_id, msg)))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "code_map".to_string(),
            display_name: "Code Map".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Documentation-worthy code map from CodeGraph and git signals: file centrality, churn hotspots, real symbol kinds and parsed visibility when present, file/module/SCC/API/infra pages, edge-derived links and backlink hubs, readiness warnings, optional hybrid query filtering, token budget trimming, and markdown or claude_md output.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional search query; when supplied, hybrid search returns only the most relevant selected pages."
                    },
                    "format": {
                        "type": "string",
                        "enum": ["markdown", "claude_md"],
                        "description": "Output format. Defaults to markdown."
                    },
                    "budget": {
                        "type": "integer",
                        "description": "Approximate token budget for selected page output. Defaults to 12000."
                    }
                }
            }),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature, Time};

    #[test]
    fn scoped_project_roots_use_worktree_for_git_and_source_for_graph_keys() {
        let temp = tempfile::tempdir().unwrap();
        let worktree_root = temp.path().join("worktree");
        let source_root = temp.path().join("source");
        std::fs::create_dir_all(worktree_root.join("src")).unwrap();
        std::fs::create_dir_all(source_root.join("src")).unwrap();
        let worktree = crate::worktrees::types::WorktreeMeta {
            id: "codegraph-scope".to_string(),
            kind: "chat".to_string(),
            root: worktree_root.clone(),
            source_workspace_root: source_root.clone(),
            repo_root: source_root.clone(),
            branch: Some("feature/codegraph".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: None,
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: true,
        };
        let scope = crate::worktrees::scope::ExecutionScope::from_worktree(&worktree);

        let roots = scoped_codegraph_project_roots(Some(&scope))
            .unwrap()
            .unwrap();

        assert_eq!(
            roots.git_root,
            dunce::simplified(&std::fs::canonicalize(worktree_root).unwrap()).to_path_buf()
        );
        assert_eq!(
            roots.indexed_root,
            dunce::simplified(&std::fs::canonicalize(source_root).unwrap()).to_path_buf()
        );

        let missing_source_path = scope
            .resolve_creatable_path(Path::new("src/deleted.rs"))
            .unwrap();
        assert_eq!(
            scoped_indexed_path(&scope, &missing_source_path.path).unwrap(),
            roots.indexed_root.join("src/deleted.rs")
        );
        let source_absolute = roots.indexed_root.join("src/deleted.rs");
        let remapped_source_path = scope.resolve_creatable_path(&source_absolute).unwrap();
        assert_eq!(remapped_source_path.path, missing_source_path.path);
        assert_eq!(
            scoped_indexed_path(&scope, &remapped_source_path.path).unwrap(),
            source_absolute
        );
        assert!(scoped_indexed_path(&scope, temp.path().join("outside.rs").as_path()).is_err());
    }

    fn commit_file(repo: &Repository, path: &str, contents: &str, msg: &str) -> git2::Oid {
        commit_file_at(
            repo,
            path,
            contents,
            msg,
            "Tester",
            "tester@example.com",
            1_700_000_000,
        )
    }

    fn commit_file_at(
        repo: &Repository,
        path: &str,
        contents: &str,
        msg: &str,
        name: &str,
        email: &str,
        ts: i64,
    ) -> git2::Oid {
        let workdir = repo.workdir().unwrap();
        let full_path = workdir.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, contents).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let time = Time::new(ts, 0);
        let sig = Signature::new(name, email, &time).unwrap();
        let parents: Vec<git2::Commit> = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok())
            .into_iter()
            .collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .unwrap()
    }

    fn two_branch_repo() -> (tempfile::TempDir, git2::Oid) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let main_oid = commit_file(&repo, "file.txt", "main\n", "main branch history");
        let main_commit = repo.find_commit(main_oid).unwrap();
        repo.branch("side", &main_commit, false).unwrap();
        repo.set_head("refs/heads/side").unwrap();
        commit_file(&repo, "file.txt", "side\n", "side branch history");
        repo.set_head("refs/heads/main").unwrap();
        (dir, main_oid)
    }

    fn code_map_node_record(
        id: i64,
        kind: &str,
        name: &str,
        path: &str,
    ) -> (i64, String, String, String, Option<String>) {
        (
            id,
            kind.to_string(),
            name.to_string(),
            path.to_string(),
            None,
        )
    }

    fn code_map_centrality(
        pagerank: &[(&str, f64)],
        betweenness: &[(&str, f64)],
    ) -> refact_codegraph::analytics::FileCentrality {
        refact_codegraph::analytics::FileCentrality {
            top_pagerank: pagerank
                .iter()
                .map(|(path, score)| ((*path).to_string(), *score))
                .collect(),
            top_betweenness: betweenness
                .iter()
                .map(|(path, score)| ((*path).to_string(), *score))
                .collect(),
        }
    }

    fn code_map_markdown_args(budget_tokens: usize) -> CodeMapArgs {
        CodeMapArgs {
            query: None,
            format: CodeMapFormat::Markdown,
            budget_tokens,
        }
    }

    fn code_map_output_fixture(args: CodeMapArgs) -> String {
        assemble_code_map_output(
            vec![
                (
                    "payments/api.rs".to_string(),
                    "pub fn charge() { validate(); }\n".to_string(),
                ),
                (
                    "auth/login.rs".to_string(),
                    "pub fn login() {}\n".to_string(),
                ),
            ],
            vec![
                code_map_node_record(1, "function", "charge", "payments/api.rs"),
                code_map_node_record(2, "function", "validate", "payments/api.rs"),
                code_map_node_record(3, "function", "login", "auth/login.rs"),
            ],
            vec![(1, 2, "calls".to_string())],
            code_map_centrality(&[("payments/api.rs", 3.0), ("auth/login.rs", 1.0)], &[]),
            Some(PrBlastIndexState {
                queued: 0,
                cross_file_edges: 1,
                cross_file_ready: true,
            }),
            None,
            None,
            &args,
        )
    }

    #[test]
    fn code_map_marks_real_hotspot() {
        let mut intel = refact_git_intel::GitIntel::default();
        intel.file_churn.insert("src/hot.rs".to_string(), 20);
        for index in 0..10 {
            intel.file_churn.insert(format!("src/cold{index}.rs"), 1);
        }
        let files_text = vec![
            ("src/hot.rs".to_string(), "fn hot() {}\n".to_string()),
            ("src/cold0.rs".to_string(), "fn cold() {}\n".to_string()),
        ];
        let nodes = code_map_nodes_from_records(vec![
            code_map_node_record(1, "function", "hot", "src/hot.rs"),
            code_map_node_record(2, "function", "cold", "src/cold0.rs"),
        ]);
        let centrality = code_map_centrality(&[], &[]);
        let files = build_code_map_files(&files_text, &nodes, &centrality, Some(&intel), None);

        let hot = files
            .iter()
            .find(|file| file.parsed.file_info.path == "src/hot.rs")
            .unwrap();
        let cold = files
            .iter()
            .find(|file| file.parsed.file_info.path == "src/cold0.rs")
            .unwrap();
        assert!(hot.is_hotspot);
        assert!(!cold.is_hotspot);

        let input = code_map_selection_input(files, &nodes, &[], &centrality);
        let pages = refact_codewiki::select_pages(&input);
        let hot_score = pages
            .iter()
            .find(|page| page.id == "file:src/hot.rs")
            .unwrap()
            .score;
        let cold_score = pages
            .iter()
            .find(|page| page.id == "file:src/cold0.rs")
            .unwrap()
            .score;
        assert!(hot_score > cold_score);
    }

    #[test]
    fn code_map_symbols_use_node_kinds() {
        let nodes = code_map_nodes_from_records(vec![
            code_map_node_record(1, "struct", "Thing", "src/model.rs"),
            code_map_node_record(2, "function", "build", "src/model.rs"),
            code_map_node_record(3, "typealias", "Id", "src/model.rs"),
        ]);
        let symbols = code_map_symbols_by_path(&nodes);
        let symbols = symbols.get("src/model.rs").unwrap();
        let kinds = symbols
            .iter()
            .map(|symbol| symbol.kind.as_str())
            .collect::<BTreeSet<_>>();

        assert!(kinds.contains("struct"));
        assert!(kinds.contains("function"));
        assert!(kinds.contains("type_alias"));
        assert!(symbols.iter().all(|symbol| symbol.visibility.is_empty()));

        let output = assemble_code_map_output(
            vec![(
                "src/model.rs".to_string(),
                "struct Thing; fn build() {}\n".to_string(),
            )],
            vec![
                code_map_node_record(1, "struct", "Thing", "src/model.rs"),
                code_map_node_record(2, "function", "build", "src/model.rs"),
                code_map_node_record(3, "typealias", "Id", "src/model.rs"),
            ],
            Vec::new(),
            code_map_centrality(&[("src/model.rs", 1.0)], &[]),
            None,
            None,
            None,
            &code_map_markdown_args(1_000),
        );
        assert!(output.contains("function=1"));
        assert!(output.contains("struct=1"));
        assert!(output.contains("type_alias=1"));
        assert!(!output.contains("public"));
    }

    #[test]
    fn code_map_links_from_graph_edges() {
        let output = assemble_code_map_output(
            vec![
                ("src/a.rs".to_string(), "fn a() { b(); }\n".to_string()),
                ("src/b.rs".to_string(), "fn b() {}\n".to_string()),
            ],
            vec![
                code_map_node_record(1, "function", "a", "src/a.rs"),
                code_map_node_record(2, "function", "b", "src/b.rs"),
            ],
            vec![(1, 2, "calls".to_string())],
            code_map_centrality(&[("src/a.rs", 2.0), ("src/b.rs", 1.0)], &[]),
            None,
            None,
            None,
            &code_map_markdown_args(2_000),
        );

        assert!(output.contains("Links:"));
        assert!(output.contains("src/b.rs (calls"));
        assert!(output.contains("Most-referenced files"));
        assert!(!output.contains("Related files:"));
    }

    #[test]
    fn code_map_query_returns_relevant_page() {
        let mut args = code_map_markdown_args(2_000);
        args.query = Some("payments".to_string());

        let output = code_map_output_fixture(args);

        assert!(output.contains("Query: `payments`"));
        assert!(output.contains("payments/api.rs"));
        assert!(!output.contains("auth/login.rs"));
    }

    #[test]
    fn claude_md_format_renders() {
        let output = code_map_output_fixture(CodeMapArgs {
            query: Some("payments".to_string()),
            format: CodeMapFormat::ClaudeMd,
            budget_tokens: 4_000,
        });

        assert!(output.contains(refact_codewiki::claude_md::BEGIN_MARKER));
        assert!(output.contains("### Selected Pages"));
        assert!(output.contains("payments/api.rs"));
    }

    #[test]
    fn budget_bounds_total_tokens() {
        let budget = 40;
        let output = code_map_output_fixture(code_map_markdown_args(budget));

        assert!(
            refact_codewiki::token_budget::estimate_tokens(&output) <= budget,
            "estimated tokens exceeded budget: {} > {}\n{}",
            refact_codewiki::token_budget::estimate_tokens(&output),
            budget,
            output
        );
    }

    #[test]
    fn git_mining_cache_does_not_store_new_head_results_under_old_head_key() {
        let (dir, main_oid) = two_branch_repo();
        let repo_path = dir.path();
        let max = 10;
        let key = (repo_path.to_path_buf(), Some(main_oid), max);
        assert_eq!(git_head_oid(repo_path).unwrap(), Some(main_oid));

        let history = cached_mine_history_with(repo_path, max, |path, _head, _base, max| {
            let repo = Repository::open(path).unwrap();
            repo.set_head("refs/heads/side").unwrap();
            refact_git_intel::mine_history(path, max)
        })
        .unwrap();

        assert_eq!(history.commit_records[0].message, "side branch history");
        assert!(git_cache().lock().unwrap().history.get(&key).is_none());
    }

    #[test]
    fn code_why_assembles_multi_kind_sources() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file(
            &repo,
            "src/a.rs",
            "a\n",
            "introduce codegraph index because symbol search needs persisted edges",
        );
        commit_file(
            &repo,
            "src/b.rs",
            "b\n",
            "switch to codegraph instead of ast maps because cross-file search needs edges",
        );
        commit_file(
            &repo,
            "src/c.rs",
            "c\n",
            "replace codegraph queue polling because incremental indexing needs lower latency",
        );
        commit_file(
            &repo,
            "src/d.rs",
            "d\n",
            "Merge branch feature/codegraph\n\n## Why\nSwitch codegraph traversal to sqlite because graph queries need persistence.",
        );
        let adr_path = dir.path().join("docs/adr/001-codegraph.md");
        std::fs::create_dir_all(adr_path.parent().unwrap()).unwrap();
        std::fs::write(
            &adr_path,
            "# Use codegraph for symbol search\n\n## Decision\nUse codegraph because symbol search needs relationships.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("CHANGELOG.md"),
            "## Unreleased\n\n### Changed\n- switch codegraph results to evidence-ranked output\n",
        )
        .unwrap();

        let intel = refact_git_intel::mine_history(dir.path(), 20).unwrap();
        let mut sources = git_code_why_sources(&intel);
        for (order, (relative, kind)) in code_why_file_kinds(dir.path()).into_iter().enumerate() {
            sources.push(CodeWhySource {
                kind: kind.to_string(),
                text: std::fs::read_to_string(dir.path().join(&relative)).unwrap(),
                reference: path_ref(&relative),
                priority: code_why_priority(kind),
                order: usize::MAX / 2 + order,
            });
        }
        let sources = cap_code_why_sources(sources);
        let kinds: BTreeSet<String> = sources.iter().map(|source| source.kind.clone()).collect();

        assert!(kinds.contains("commit"));
        assert!(kinds.contains("pr"));
        assert!(kinds.contains("adr"));
        assert!(kinds.contains("changelog"));
        assert_eq!(
            sources.first().map(|source| source.kind.as_str()),
            Some("adr")
        );
    }

    #[test]
    fn code_why_finds_prose_decision_end_to_end() {
        let statement =
            "switch codegraph storage to sqlite instead of memory because graph queries need persistence";
        let sources = vec![
            CodeWhySource {
                kind: "adr".to_string(),
                text: format!("# {statement}\n\n## Decision\n{statement}\n"),
                reference: "docs/adr/001-codegraph.md".to_string(),
                priority: code_why_priority("adr"),
                order: 0,
            },
            CodeWhySource {
                kind: "commit".to_string(),
                text: statement.to_string(),
                reference: "abcdef123456".to_string(),
                priority: code_why_priority("commit"),
                order: 1,
            },
        ];

        let output = code_why_output("codegraph", &sources, 2).unwrap();

        assert!(output.contains(statement));
        assert!(output.contains("[verbatim"));
        assert!(output.contains("corr=2"));
        assert!(output.contains("src=adr:docs/adr/001-codegraph.md"));
    }

    #[test]
    fn code_why_file_sources_bounded_and_prioritized() {
        let mut git_sources = Vec::new();
        for index in 0..CODE_WHY_SOURCE_LIMIT {
            git_sources.push(CodeWhySource {
                kind: "commit".to_string(),
                text: format!("commit decision {index}"),
                reference: format!("commit:{index:03}"),
                priority: code_why_priority("commit"),
                order: index,
            });
        }
        git_sources.push(CodeWhySource {
            kind: "pr".to_string(),
            text: "merge decision".to_string(),
            reference: "pr:001".to_string(),
            priority: code_why_priority("pr"),
            order: CODE_WHY_SOURCE_LIMIT,
        });
        let candidates = vec![
            CodeWhyFileCandidate {
                relative: PathBuf::from("docs/adr/002.md"),
                kind: "adr",
                reference: "docs/adr/002.md".to_string(),
                priority: code_why_priority("adr"),
                order: usize::MAX / 2 + 1,
            },
            CodeWhyFileCandidate {
                relative: PathBuf::from("CHANGELOG.md"),
                kind: "changelog",
                reference: "CHANGELOG.md".to_string(),
                priority: code_why_priority("changelog"),
                order: usize::MAX / 2 + 2,
            },
            CodeWhyFileCandidate {
                relative: PathBuf::from("docs/adr/001.md"),
                kind: "adr",
                reference: "docs/adr/001.md".to_string(),
                priority: code_why_priority("adr"),
                order: usize::MAX / 2,
            },
        ];

        let selected =
            select_code_why_file_candidates(candidates, &git_sources, CODE_WHY_SOURCE_LIMIT);
        let selected_refs = selected
            .iter()
            .map(|candidate| candidate.reference.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            selected_refs,
            vec!["docs/adr/001.md", "docs/adr/002.md", "CHANGELOG.md"]
        );

        let mut sources = git_sources;
        sources.extend(selected.into_iter().map(|candidate| CodeWhySource {
            kind: candidate.kind.to_string(),
            text: candidate.reference.clone(),
            reference: candidate.reference,
            priority: candidate.priority,
            order: candidate.order,
        }));
        let sources = cap_code_why_sources(sources);
        let first_refs = sources
            .iter()
            .take(5)
            .map(|source| source.reference.as_str())
            .collect::<Vec<_>>();

        assert_eq!(sources.len(), CODE_WHY_SOURCE_LIMIT);
        assert_eq!(
            first_refs,
            vec![
                "docs/adr/001.md",
                "docs/adr/002.md",
                "CHANGELOG.md",
                "pr:001",
                "commit:000"
            ]
        );
        assert!(!sources
            .iter()
            .any(|source| source.reference == "commit:796"));
        assert!(!sources
            .iter()
            .any(|source| source.reference == "commit:797"));
        assert!(!sources
            .iter()
            .any(|source| source.reference == "commit:798"));
        assert!(!sources
            .iter()
            .any(|source| source.reference == "commit:799"));
    }

    fn allow_all_privacy(gcx: &Arc<crate::global_context::GlobalContext>) {
        let loaded_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: Vec::new(),
            },
            loaded_ts,
        });
    }

    #[tokio::test]
    async fn assemble_code_why_sources_bounds_file_reads_before_reading() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file(
            &repo,
            "src/a.rs",
            "a\n",
            "switch code why source selection because file reads need bounds",
        );
        std::fs::create_dir_all(dir.path().join("docs/adr")).unwrap();
        for index in 0..(CODE_WHY_SOURCE_LIMIT + 20) {
            std::fs::write(
                dir.path().join(format!("docs/adr/{index:04}.md")),
                format!("# ADR {index}\n\nDecision {index}\n"),
            )
            .unwrap();
        }
        std::fs::write(dir.path().join("CHANGELOG.md"), "# Changelog\n").unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let intel = refact_git_intel::mine_history(dir.path(), 20).unwrap();
        let git_sources = git_code_why_sources(&intel);
        let candidate_count = code_why_file_candidates(dir.path()).len();
        let selected_before_reads = code_why_selected_candidate_count_before_reads(
            dir.path(),
            &git_sources,
            CODE_WHY_SOURCE_LIMIT,
        );
        CODE_WHY_FILE_READ_ATTEMPTS.store(0, std::sync::atomic::Ordering::Relaxed);

        let sources = assemble_code_why_sources(gcx, dir.path(), &intel).await;
        let read_attempts = CODE_WHY_FILE_READ_ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed);

        assert!(candidate_count > CODE_WHY_SOURCE_LIMIT);
        assert!(selected_before_reads <= CODE_WHY_SOURCE_LIMIT);
        assert_eq!(read_attempts, selected_before_reads);
        assert!(sources.len() <= CODE_WHY_SOURCE_LIMIT);
        let first_kinds = sources
            .iter()
            .take(4)
            .map(|source| source.kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(first_kinds, vec!["adr", "adr", "adr", "adr"]);
        let first_non_adr = sources
            .iter()
            .position(|source| source.kind != "adr")
            .unwrap_or(sources.len());
        let changelog_pos = sources.iter().position(|source| source.kind == "changelog");
        let pr_pos = sources.iter().position(|source| source.kind == "pr");
        let commit_pos = sources.iter().position(|source| source.kind == "commit");
        if let Some(changelog_pos) = changelog_pos {
            assert!(sources[..changelog_pos]
                .iter()
                .all(|source| source.kind == "adr"));
            if let Some(pr_pos) = pr_pos {
                assert!(changelog_pos < pr_pos);
            }
            if let Some(commit_pos) = commit_pos {
                assert!(changelog_pos < commit_pos);
            }
        } else {
            assert_eq!(first_non_adr, sources.len());
        }
        if let (Some(pr_pos), Some(commit_pos)) = (pr_pos, commit_pos) {
            assert!(pr_pos < commit_pos);
        }
    }

    #[test]
    fn cached_mine_history_incremental_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file(
            &repo,
            "src/a.rs",
            "a\n",
            "introduce codegraph history because decisions need provenance",
        );

        let first = cached_mine_history_with(dir.path(), 20, |path, _head, base, max| {
            assert!(base.is_none());
            refact_git_intel::mine_history_incremental(path, base, max)
        })
        .unwrap();
        let first_head = first.last_commit_id.clone();
        commit_file(
            &repo,
            "src/b.rs",
            "b\n",
            "switch codegraph history to incremental because repeated mining is expensive",
        );

        let second = cached_mine_history_with(dir.path(), 20, |path, _head, base, max| {
            let base = base.expect("previous cached history is reused as incremental base");
            assert_eq!(base.last_commit_id, first_head);
            refact_git_intel::mine_history_incremental(path, Some(base), max)
        })
        .unwrap();

        assert_eq!(second.commits_analyzed, first.commits_analyzed + 1);
        assert_ne!(second.last_commit_id, first.last_commit_id);
    }

    #[test]
    #[ignore]
    fn code_why_real_repo_smoke() {
        let repo_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let intel = refact_git_intel::mine_history(repo_path, CODE_WHY_HISTORY_COMMITS).unwrap();
        let sources = git_code_why_sources(&intel);
        let output = code_why_output("codegraph", &sources, intel.commits_analyzed).unwrap();

        assert!(output.contains("codegraph"));
        println!("{}", output.lines().take(8).collect::<Vec<_>>().join("\n"));
    }

    #[test]
    fn pr_blast_reviewers_are_ownership_based_stable_and_bot_filtered() {
        let report = refact_codegraph::pr_blast::BlastReport {
            changed_files: vec!["/repo/src/core.rs".to_string()],
            directly_impacted: vec![refact_codegraph::pr_blast::BlastImpact {
                path: "src/caller.rs".to_string(),
                symbol: "caller".to_string(),
                distance: 1,
                via: "calls".to_string(),
                kind: refact_codegraph::pr_blast::ImpactKind::Behavioral,
            }],
            transitively_impacted: Vec::new(),
            impacted_file_count: 1,
            risk_score: 0.1,
        };
        let mut intel = refact_git_intel::GitIntel::default();
        intel.file_authors.insert(
            "src/core.rs".to_string(),
            HashMap::from([
                ("b@example.com".to_string(), 1),
                ("a@example.com".to_string(), 1),
                ("builder[bot]@users.noreply.github.com".to_string(), 10),
            ]),
        );
        intel.file_authors.insert(
            "src/caller.rs".to_string(),
            HashMap::from([
                ("b@example.com".to_string(), 1),
                ("a@example.com".to_string(), 1),
                ("review-agent@example.com".to_string(), 10),
            ]),
        );

        let reviewers =
            pr_blast_suggested_reviewers(&report, Some(Path::new("/repo")), Some(&intel));

        assert_eq!(reviewers.len(), 2);
        assert_eq!(reviewers[0].author, "a@example.com");
        assert_eq!(reviewers[1].author, "b@example.com");
        assert!((reviewers[0].score - reviewers[1].score).abs() < 1e-9);
    }

    #[test]
    fn pr_blast_reviewers_are_empty_without_repo_or_intel() {
        let report = refact_codegraph::pr_blast::BlastReport {
            changed_files: vec!["src/core.rs".to_string()],
            directly_impacted: Vec::new(),
            transitively_impacted: Vec::new(),
            impacted_file_count: 0,
            risk_score: 0.0,
        };
        let intel = refact_git_intel::GitIntel::default();

        assert!(pr_blast_suggested_reviewers(&report, None, Some(&intel)).is_empty());
        assert!(pr_blast_suggested_reviewers(&report, Some(Path::new("/repo")), None).is_empty());
        assert!(pr_blast_is_bot_author("robot[bot]@example.com"));
        assert!(pr_blast_is_bot_author("review-agent@example.com"));
        assert!(pr_blast_is_bot_author(
            "123+noreply@users.noreply.github.com"
        ));
    }

    fn branchy_hotspot_source(a: i32, b: i32, c: i32) -> String {
        format!(
            "pub fn hot(x: i32) -> i32 {{\n    if x > 0 {{\n        if x > 1 {{\n            for i in 0..x {{\n                if i % 2 == 0 {{\n                    return {a};\n                }}\n            }}\n            return {b};\n        }}\n    }}\n    {c}\n}}\n"
        )
    }

    fn brain_method_source(caller_count: usize) -> String {
        let mut src = "fn brain(x: i32) -> i32 {\n".to_string();
        for i in 0..75 {
            src.push_str(&format!(
                "    if x > {i} {{ if x % 2 == 0 {{ if x < 100 {{ if x != 42 {{ return {i}; }} }} }} }}\n"
            ));
        }
        src.push_str("    0\n}\n");
        for i in 0..caller_count {
            src.push_str(&format!("fn caller_{i}(x: i32) -> i32 {{ brain(x) }}\n"));
        }
        src
    }

    async fn service_with_file(
        path: &str,
        text: &str,
        lang: &str,
    ) -> Arc<refact_codegraph::CodeGraphService> {
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        service.index_file(path, text, lang).await.unwrap();
        service.connect_usages().await.unwrap();
        service
    }

    #[tokio::test]
    async fn brain_method_downgraded_at_zero_fanin() {
        let text = brain_method_source(0);
        let service = service_with_file("src/a.rs", &text, "rust").await;
        let health = refact_codehealth::analyze("rust", &text);
        let findings = refact_codehealth::biomarkers::detect_biomarkers("rust", &text);
        let original = findings
            .iter()
            .find(|finding| finding.biomarker == "brain_method")
            .unwrap()
            .severity;

        let enriched =
            enrich_health_findings(Some(&service), "src/a.rs", &health.functions, findings).await;
        let brain = enriched
            .iter()
            .find(|finding| finding.finding.biomarker == "brain_method")
            .unwrap();

        assert_eq!(brain.finding.severity, downgrade_health_severity(original));
        assert!(brain.finding.detail.contains("low fan-in"));
    }

    #[tokio::test]
    async fn annotated_at_high_fanin() {
        let text = brain_method_source(5);
        let service = service_with_file("src/a.rs", &text, "rust").await;
        let health = refact_codehealth::analyze("rust", &text);
        let findings = refact_codehealth::biomarkers::detect_biomarkers("rust", &text);
        let original = findings
            .iter()
            .find(|finding| finding.biomarker == "brain_method")
            .unwrap()
            .severity;

        let enriched =
            enrich_health_findings(Some(&service), "src/a.rs", &health.functions, findings).await;
        let brain = enriched
            .iter()
            .find(|finding| finding.finding.biomarker == "brain_method")
            .unwrap();

        assert_eq!(brain.finding.severity, original);
        assert!(brain.finding.detail.contains("called from 5 sites"));
    }

    #[tokio::test]
    async fn perf_finding_hot_path_from_route() {
        let text = r#"
from fastapi import FastAPI
import requests
app = FastAPI()

@app.get("/users")
def handler():
    return helper()

def helper():
    for u in urls:
        for p in pages:
            requests.get(p)

def orphan():
    for u in urls:
        for p in pages:
            requests.get(p)
"#;
        let service = service_with_file("src/api.py", text, "python").await;
        let health = refact_codehealth::analyze("python", text);
        let findings = refact_codehealth::perf::detect_perf("python", text);
        let helper = health
            .functions
            .iter()
            .find(|function| function.name == "helper")
            .unwrap();
        let orphan = health
            .functions
            .iter()
            .find(|function| function.name == "orphan")
            .unwrap();

        let enriched =
            enrich_health_findings(Some(&service), "src/api.py", &health.functions, findings).await;

        assert!(enriched.iter().any(|finding| {
            finding.hot_path
                && finding.finding.dimension
                    == refact_codehealth::biomarkers::Dimension::Performance
                && helper.line1 <= finding.finding.line
                && finding.finding.line <= function_line2(helper)
        }));
        assert!(enriched.iter().any(|finding| {
            !finding.hot_path
                && finding.finding.dimension
                    == refact_codehealth::biomarkers::Dimension::Performance
                && orphan.line1 <= finding.finding.line
                && finding.finding.line <= function_line2(orphan)
        }));
    }

    #[tokio::test]
    async fn build_git_meta_fills_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file_at(
            &repo,
            "src/a.rs",
            &branchy_hotspot_source(1, 1, 1),
            "introduce hot",
            "Alice",
            "alice@example.com",
            1_600_000_000,
        );
        commit_file_at(
            &repo,
            "src/a.rs",
            &branchy_hotspot_source(2, 1, 1),
            "fix crash in hot",
            "Bob",
            "bob@example.com",
            1_600_000_100,
        );
        commit_file_at(
            &repo,
            "src/a.rs",
            &branchy_hotspot_source(2, 2, 1),
            "update hot",
            "Carol",
            "carol@example.com",
            1_600_000_200,
        );
        let intel = refact_git_intel::mine_history(dir.path(), 20).unwrap();
        let ranges = vec![("hot".to_string(), 1, 11)];

        let meta = build_git_meta(&intel, dir.path(), "src/a.rs", None, Some(&ranges)).await;

        assert_eq!(meta.prior_defect_count, 1);
        assert!(meta.is_stable);
        assert_eq!(meta.functions.len(), 1);
        assert_eq!(meta.functions[0].name, "hot");
        assert!(meta.functions[0].modification_count >= 3);
        assert!(meta.functions[0].ccn >= 4);
        assert!(meta.repo_function_mod_p80.is_some());
        assert_eq!(meta.repo_commit_counts.get("src/a.rs"), Some(&3));
    }

    #[tokio::test]
    async fn hidden_coupling_fires_without_import_edge() {
        let dir = tempfile::tempdir().unwrap();
        let mut intel = refact_git_intel::GitIntel::default();
        intel.file_churn.insert("src/a.rs".into(), 6);
        intel.file_churn.insert("src/b.rs".into(), 6);
        intel
            .co_change
            .insert(("src/a.rs".into(), "src/b.rs".into()), 6);
        intel.commit_records = (0..6)
            .map(|i| refact_git_intel::CommitRecord {
                oid: Some(format!("c{i}")),
                ts: 1_700_000_000 + i,
                author: "dev@example.com".into(),
                committer: "dev@example.com".into(),
                message: format!("change {i}"),
                files: vec![("src/a.rs".into(), 1, 0), ("src/b.rs".into(), 1, 0)],
            })
            .collect();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn a() {}\n").unwrap();

        let meta = build_git_meta(&intel, dir.path(), "src/a.rs", None, None).await;
        let findings = refact_codehealth::git_biomarkers::git_biomarkers(&meta);

        assert!(findings
            .iter()
            .any(|finding| finding.biomarker == "hidden_coupling"));
    }

    #[tokio::test]
    async fn suppressed_with_import_edge() {
        let dir = tempfile::tempdir().unwrap();
        let mut intel = refact_git_intel::GitIntel::default();
        intel.file_churn.insert("src/a.rs".into(), 6);
        intel.file_churn.insert("src/b.rs".into(), 6);
        intel
            .co_change
            .insert(("src/a.rs".into(), "src/b.rs".into()), 6);
        intel.commit_records = (0..6)
            .map(|i| refact_git_intel::CommitRecord {
                oid: Some(format!("c{i}")),
                ts: 1_700_000_000 + i,
                author: "dev@example.com".into(),
                committer: "dev@example.com".into(),
                message: format!("change {i}"),
                files: vec![("src/a.rs".into(), 1, 0), ("src/b.rs".into(), 1, 0)],
            })
            .collect();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn a() { b(); }\n").unwrap();
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file("src/a.rs", "fn a() { b(); }\n", "rust")
            .await
            .unwrap();
        service
            .index_file("src/b.rs", "fn b() {}\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();

        let meta = build_git_meta(&intel, dir.path(), "src/a.rs", Some(&service), None).await;
        let findings = refact_codehealth::git_biomarkers::git_biomarkers(&meta);

        assert!(meta.import_edges.contains("src/b.rs"));
        assert!(!findings
            .iter()
            .any(|finding| finding.biomarker == "hidden_coupling"));
    }

    #[tokio::test]
    async fn build_git_risk_assembly_reaches_dead_detectors() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file_at(
            &repo,
            "src/a.rs",
            &branchy_hotspot_source(1, 1, 1),
            "introduce hot",
            "Alice",
            "alice@example.com",
            1_600_000_000,
        );
        commit_file_at(
            &repo,
            "src/a.rs",
            &branchy_hotspot_source(2, 1, 1),
            "fix crash in hot",
            "Bob",
            "bob@example.com",
            1_600_000_100,
        );
        commit_file_at(
            &repo,
            "src/a.rs",
            &branchy_hotspot_source(2, 2, 1),
            "fix regression in hot",
            "Carol",
            "carol@example.com",
            1_600_000_200,
        );
        commit_file_at(
            &repo,
            "src/a.rs",
            &branchy_hotspot_source(2, 2, 2),
            "update hot again",
            "Dana",
            "dana@example.com",
            1_600_000_300,
        );
        let intel = refact_git_intel::mine_history(dir.path(), 20).unwrap();

        let assembly = build_git_risk_assembly(&intel, dir.path(), None, 1, None).await;
        let finding_names = assembly.files[0]
            .findings
            .iter()
            .map(|finding| finding.biomarker.as_str())
            .collect::<HashSet<_>>();

        assert!(finding_names.contains("prior_defect"));
        assert!(finding_names.contains("function_hotspot"));
        assert!(finding_names.contains("code_age_volatility"));
    }

    fn shared_clone_body(name: &str) -> String {
        let mut src = format!("fn {name}(input: i32) -> i32 {{\n    let mut total = input;\n");
        for _ in 0..30 {
            src.push_str("    total = total + input;\n    total = total - input;\n");
        }
        src.push_str("    total\n}\n");
        src
    }

    #[test]
    fn git_risk_tool_and_http_share_assembly_text() {
        let mut intel = refact_git_intel::GitIntel::default();
        intel.file_churn.insert("src/a.rs".into(), 3);
        intel.file_authors.insert(
            "src/a.rs".into(),
            HashMap::from([
                ("alice@example.com".into(), 2),
                ("bob@example.com".into(), 1),
            ]),
        );
        intel.commit_records = vec![refact_git_intel::CommitRecord {
            oid: Some("abcdef1234567890".into()),
            ts: 1,
            author: "alice@example.com".into(),
            committer: "alice@example.com".into(),
            message: "large risky change".into(),
            files: vec![("src/a.rs".into(), 200, 10)],
        }];
        let meta = refact_codehealth::git_biomarkers::GitMeta {
            file_path: "src/a.rs".into(),
            change_entropy: 0.0,
            change_entropy_pct: 0.0,
            commit_count_90d: 1,
            commit_count_total: 3,
            is_hotspot: true,
            is_stable: false,
            churn_percentile: 1.0,
            lines_added_90d: 200,
            lines_deleted_90d: 10,
            nloc: 20,
            contributor_count: 2,
            primary_owner_commit_pct: 0.67,
            primary_owner_name: "alice@example.com".into(),
            recent_owner_name: "alice@example.com".into(),
            recent_owner_commit_pct: 1.0,
            bus_factor: 2,
            prior_defect_count: 1,
            repo_active_contributors_90d: Some(2),
            repo_function_mod_p80: None,
            co_change_partners: Vec::new(),
            top_authors: vec![
                ("alice@example.com".into(), 2),
                ("bob@example.com".into(), 1),
            ],
            functions: Vec::new(),
            repo_commit_counts: HashMap::from([("src/a.rs".into(), 3)]),
            import_edges: HashSet::new(),
        };
        let findings = refact_codehealth::git_biomarkers::git_biomarkers(&meta);
        let assembly = GitRiskAssembly {
            files: vec![GitRiskFileAssembly {
                meta,
                findings,
                temporal_score: 1.0,
                churn_risk: 0.5,
            }],
            recent_commit_risks: recent_commit_risk_summaries(&intel),
        };

        let tool_text = git_risk_tool_output(&intel, &assembly);

        assert!(tool_text.contains("src/a.rs"));
        assert!(tool_text.contains("prior_defect"));
        assert!(tool_text.contains("Recent commit change-risk"));
    }

    fn empty_health_ctx<'a>(
        trend_findings: &'a [refact_codehealth::biomarkers::Finding],
        git_function_fact_paths: &'a HashSet<String>,
    ) -> HealthAnalysisContext<'a> {
        HealthAnalysisContext {
            repo_root: None,
            intel: None,
            service: None,
            coverage: None,
            trend_findings,
            git_function_fact_paths,
        }
    }

    fn duplicate_health_source() -> String {
        let mut body = String::new();
        for name in ["alpha", "beta"] {
            body.push_str(&format!(
                "fn {name}(input: i32) -> i32 {{\n    let mut total = input;\n"
            ));
            for _ in 0..30 {
                body.push_str("    total = total + input;\n    total = total - input;\n");
            }
            body.push_str("    total\n}\n");
        }
        body
    }

    #[test]
    fn dense_short_clone_does_not_trigger_health_dry_finding() {
        let dense_line = (0..30)
            .map(|_| "total = total + input; total = total - input;")
            .collect::<Vec<_>>()
            .join(" ");
        let src = format!(
            "fn alpha(input: i32) -> i32 {{ let mut total = input; {dense_line} total }}\nfn beta(value: i32) -> i32 {{ let mut total = value; {} total }}\n",
            dense_line.replace("input", "value")
        );
        let clones = refact_codehealth::duplication::detect_clones("rust", &src);

        assert!(
            clones.iter().any(|clone| clone.token_len >= 60
                && clone
                    .a_end_line
                    .saturating_sub(clone.line_a)
                    .saturating_add(1)
                    <= 3
                && clone
                    .b_end_line
                    .saturating_sub(clone.line_b)
                    .saturating_add(1)
                    <= 3),
            "expected dense short clone, got {clones:?}"
        );
        assert!(health_dry_findings("src/dense.rs", "rust", &src, 1.0).is_empty());
    }

    #[tokio::test]
    async fn structural_only_fixture_score_unchanged() {
        reset_health_cache_for_tests();
        let src = "fn branchy(x: i32) -> i32 {\n    if x > 0 {\n        if x > 1 {\n            return x;\n        }\n    }\n    0\n}\n";
        let trends = Vec::new();
        let git_paths = HashSet::new();
        let ctx = empty_health_ctx(&trends, &git_paths);

        let analysis = analyze_health_file_shared("src/a.rs".to_string(), src.to_string(), &ctx)
            .await
            .unwrap();
        let mut expected_findings = refact_codehealth::biomarkers::detect_biomarkers("rust", src);
        expected_findings.extend(refact_codehealth::perf::detect_perf("rust", src));
        let expected = refact_codehealth::scoring::score_file(&expected_findings);

        assert_eq!(analysis.defect_score, expected.defect);
        assert_eq!(analysis.maintainability_score, expected.maintainability);
        assert_eq!(analysis.performance_score, expected.performance);
    }

    #[tokio::test]
    async fn health_cache_hits_on_unchanged_content_and_invalidates_on_edit() {
        reset_health_cache_for_tests();
        let trends = Vec::new();
        let git_paths = HashSet::new();
        let ctx = empty_health_ctx(&trends, &git_paths);
        let src = "fn a() -> i32 { 1 }\n".to_string();

        let first = analyze_health_file_shared(
            "src/health_cache_probe_unique.rs".to_string(),
            src.clone(),
            &ctx,
        )
        .await
        .unwrap();
        let second =
            analyze_health_file_shared("src/health_cache_probe_unique.rs".to_string(), src, &ctx)
                .await
                .unwrap();
        let third = analyze_health_file_shared(
            "src/health_cache_probe_unique.rs".to_string(),
            "fn a() -> i32 { if true { 1 } else { 0 } }\n".to_string(),
            &ctx,
        )
        .await
        .unwrap();

        assert!(!first.cache_hit);
        assert!(second.cache_hit);
        assert!(!third.cache_hit);
        // NOTE: no global-stats assertion here — the cache/counters are process-global
        // and other parallel tests also exercise analyze_health_file_shared; the three
        // per-call cache_hit flags above are the race-free proof of hit/invalidate.
    }

    #[tokio::test]
    async fn health_scores_include_git_and_dry_categories() {
        reset_health_cache_for_tests();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let src = duplicate_health_source();
        std::fs::write(dir.path().join("src/a.rs"), &src).unwrap();
        let mut intel = refact_git_intel::GitIntel::default();
        intel.file_churn.insert("src/a.rs".to_string(), 6);
        intel.fix_commit_counts.insert("src/a.rs".to_string(), 3);
        intel.commits_analyzed = 6;
        let trends = Vec::new();
        let git_paths = top_git_function_fact_paths(Some(&intel));
        let ctx = HealthAnalysisContext {
            repo_root: Some(dir.path()),
            intel: Some(&intel),
            service: None,
            coverage: None,
            trend_findings: &trends,
            git_function_fact_paths: &git_paths,
        };

        let analysis = analyze_health_file_shared("src/a.rs".to_string(), src, &ctx)
            .await
            .unwrap();
        let categories = analysis
            .findings
            .iter()
            .map(|finding| finding.finding.category.as_str())
            .collect::<HashSet<_>>();
        let impacts = analysis
            .health_impact
            .iter()
            .map(|impact| impact.biomarker.as_str())
            .collect::<HashSet<_>>();

        assert!(
            categories.contains("duplication"),
            "{:?}",
            analysis.findings
        );
        assert!(
            categories.contains("organizational"),
            "{:?}",
            analysis.findings
        );
        assert!(impacts.contains("prior_defect") || impacts.contains("dry_violation"));
    }

    #[tokio::test]
    async fn no_coverage_file_means_zero_coverage_findings() {
        reset_health_cache_for_tests();
        let trends = Vec::new();
        let git_paths = HashSet::new();
        let ctx = empty_health_ctx(&trends, &git_paths);

        let analysis = analyze_health_file_shared(
            "src/a.rs".to_string(),
            "fn a() -> i32 { 1 }\n".to_string(),
            &ctx,
        )
        .await
        .unwrap();

        assert!(analysis.findings.iter().all(|finding| {
            finding.finding.category != "test_coverage"
                && finding.finding.category != "coverage_gradient"
        }));
    }

    #[tokio::test]
    async fn trend_snapshot_persisted_and_trend_finding_fires() {
        reset_health_cache_for_tests();
        let service = refact_codegraph::CodeGraphService::open_in_memory().unwrap();
        let snapshots = vec![
            health_snap(1, "src/a.rs", 10.0),
            health_snap(2, "src/a.rs", 8.8),
            health_snap(3, "src/a.rs", 7.6),
        ];
        persist_health_snapshots(&service, &snapshots)
            .await
            .unwrap();
        let mut loaded = load_health_snapshots(&service).await;
        push_health_snapshot(&mut loaded, health_snap(4, "src/a.rs", 6.4));
        let trends = refact_codehealth::trends::evaluate_trends(&loaded);
        let git_paths = HashSet::new();
        let ctx = empty_health_ctx(&trends, &git_paths);

        let analysis = analyze_health_file_shared(
            "src/a.rs".to_string(),
            "fn a() -> i32 { 1 }\n".to_string(),
            &ctx,
        )
        .await
        .unwrap();
        persist_health_snapshots(&service, &loaded).await.unwrap();
        let snapshots = load_health_snapshots(&service).await;

        assert!(analysis
            .findings
            .iter()
            .any(|finding| finding.finding.biomarker == "health_declining"));
        assert_eq!(snapshots.len(), 4);
    }

    fn health_snap(ts: i64, path: &str, score: f64) -> refact_codehealth::trends::HealthSnapshot {
        refact_codehealth::trends::HealthSnapshot {
            ts,
            per_file: HashMap::from([(path.to_string(), score)]),
            aggregate: score,
        }
    }

    #[test]
    fn basename_collision_does_not_cross_bind() {
        let matched = health_stored_path_for_request(
            vec!["a/util.rs".to_string(), "b/util.rs".to_string()],
            "util.rs",
            Some(Path::new("/repo")),
        );

        assert_eq!(matched, None);
    }

    #[tokio::test]
    async fn clone_cache_reuses_until_generation_bump() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file("src/a.rs", &shared_clone_body("alpha"), "rust")
            .await
            .unwrap();
        service
            .index_file("src/b.rs", &shared_clone_body("beta"), "rust")
            .await
            .unwrap();
        *gcx.codegraph.lock().await = Some(service.clone());

        let first = cached_cross_file_clones(gcx.clone()).await.unwrap();
        let second = cached_cross_file_clones(gcx.clone()).await.unwrap();

        assert!(
            !first.clones.is_empty(),
            "expected cross-file clone between a.rs and b.rs"
        );
        assert!(
            Arc::ptr_eq(&first, &second),
            "same generation must serve the cached analysis"
        );

        service
            .index_file("src/c.rs", "fn gamma() -> i32 { 3 }\n", "rust")
            .await
            .unwrap();
        let third = cached_cross_file_clones(gcx.clone()).await.unwrap();
        assert!(
            !Arc::ptr_eq(&second, &third),
            "generation bump must rebuild the clone analysis"
        );
    }
}
