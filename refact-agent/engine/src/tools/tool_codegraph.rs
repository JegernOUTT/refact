use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use git2::{Oid, Repository};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
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
const GIT_RISK_RECENT_COMMITS: usize = 50;
const GIT_RISK_RECENT_COMMIT_LIMIT: usize = 10;

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
    pub(crate) total_candidates: usize,
    pub(crate) index_state: DeadCodeIndexState,
    pub(crate) partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    let repo_dir = project_dir(gcx.clone()).await;
    let intel = repo_dir
        .as_deref()
        .and_then(|dir| cached_mine_history(dir, DEAD_CODE_HISTORY_COMMITS).ok());
    let now_ts = current_unix_ts();
    let facts = dead_code_git_facts_by_node(&dead, repo_dir.as_deref(), intel.as_ref(), now_ts);
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

pub(crate) async fn build_git_meta(
    intel: &refact_git_intel::GitIntel,
    repo_root: &Path,
    path_repo_relative: &str,
    service: Option<&Arc<refact_codegraph::CodeGraphService>>,
    function_ranges: Option<&[(String, usize, usize)]>,
) -> refact_codehealth::git_biomarkers::GitMeta {
    let now_ts = now_unix_ts();
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
    let now_ts = now_unix_ts();
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

fn now_unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
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
            description: "Project-wide code-graph overview: node/edge counts, strongly-connected components, and the most central symbols by PageRank.".to_string(),
            input_schema: json_schema_from_params(&[], &[]),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        codegraph_dependency()
    }
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
        let gcx = ccx.lock().await.app.gcx.clone();
        let report = dead_code_report(gcx, limit, path.as_deref(), min_confidence).await?;
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
            description: "Static reachability + git-recency ladder dead-code report; overrides and script entry points excluded; confidence-ranked with indexing warnings.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("limit", "integer", "Maximum number of candidates to return. Defaults to 50, max 500."),
                    ("path", "string", "Optional path or basename filter."),
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
        let file_path = string_arg(args, "file_path")?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let text = crate::files_in_workspace::get_file_text_from_memory_or_disk(
            gcx.clone(),
            &PathBuf::from(&file_path),
        )
        .await?
        .to_string();
        let lang = refact_codegraph::lang_from_path(&file_path);
        let health = refact_codehealth::analyze(lang, &text);
        if health.functions.is_empty() {
            return Ok((
                false,
                tool_message(
                    tool_call_id,
                    format!("No functions analyzed in `{file_path}` (lang: {lang})."),
                ),
            ));
        }
        let mut msg = format!(
            "Code health for `{}` (avg maintainability {:.1}, max complexity {}):\n",
            file_path, health.avg_maintainability, health.max_complexity
        );
        let category = refact_codewiki::well_known::file_category(&file_path, lang, false);
        match refact_codewiki::well_known::well_known_role(&file_path) {
            Some(role) => msg.push_str(&format!("File role: {role} (category: {category})\n")),
            None => msg.push_str(&format!("File category: {category}\n")),
        }
        let mut funcs = health.functions.clone();
        funcs.sort_by(|a, b| b.complexity.cmp(&a.complexity));
        for f in funcs.iter().take(30) {
            msg.push_str(&format!(
                "  {}:{}  complexity={} nesting={} loc={} maintainability={:.0}\n",
                f.name, f.line1, f.complexity, f.nesting, f.loc, f.maintainability
            ));
        }
        let dup_pct = refact_codehealth::duplication::duplication_pct(lang, &text);
        if dup_pct > 0.0 {
            msg.push_str(&format!(
                "\nDuplication: {:.0}% of tokens are in clones{}\n",
                dup_pct * 100.0,
                if refact_codehealth::duplication::dry_violation(lang, &text) {
                    " (DRY violation)"
                } else {
                    ""
                }
            ));
        }
        let mut findings = refact_codehealth::biomarkers::detect_biomarkers(lang, &text);
        findings.extend(refact_codehealth::perf::detect_perf(lang, &text));
        let score = refact_codehealth::scoring::score_file(&findings);
        msg.push_str(&format!(
            "\nHealth score (1-10): defect={:.1} ({}) · maintainability={:.1} · performance={:.1}\n",
            score.defect,
            refact_codehealth::scoring::grade(score.defect),
            score.maintainability,
            score.performance
        ));
        if !findings.is_empty() {
            msg.push_str(&format!("Biomarkers ({}):\n", findings.len()));
            for fnd in findings.iter().take(15) {
                msg.push_str(&format!(
                    "  {}:{} {} [{:?}/{:?}] {}\n",
                    file_path, fnd.line, fnd.biomarker, fnd.severity, fnd.dimension, fnd.detail
                ));
            }
        }
        let suggestions = refact_codehealth::refactoring::rank(
            refact_codehealth::refactoring::suggest_refactorings(lang, &text),
        );
        if !suggestions.is_empty() {
            msg.push_str("Refactoring targets:\n");
            for r in suggestions.iter().take(8) {
                msg.push_str(&format!(
                    "  {:?} @ {}:{} — {} (impact {:.1}, {} effort)\n",
                    r.kind, file_path, r.line, r.rationale, r.impact, r.effort
                ));
            }
        }
        if let Some(service) = gcx.codegraph.lock().await.clone() {
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
                let stored_path = if cached.data.nodes.iter().any(|(_, _, p)| p == &file_path) {
                    Some(file_path.clone())
                } else {
                    let mut candidates: std::collections::BTreeSet<&str> =
                        std::collections::BTreeSet::new();
                    let requested_base = file_path.rsplit('/').next().unwrap_or(&file_path);
                    for (_, _, p) in &cached.data.nodes {
                        let base = p.rsplit('/').next().unwrap_or(p);
                        if p.ends_with(file_path.as_str())
                            || file_path.ends_with(p.as_str())
                            || base == requested_base
                        {
                            candidates.insert(p.as_str());
                        }
                    }
                    if candidates.len() == 1 {
                        candidates.iter().next().map(|s| s.to_string())
                    } else {
                        None
                    }
                };
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
        if let Some(Value::String(cov_path)) = args.get("coverage_file") {
            match crate::files_in_workspace::get_file_text_from_memory_or_disk(
                gcx.clone(),
                &PathBuf::from(cov_path),
            )
            .await
            {
                Ok(rope) => {
                    match refact_codehealth::coverage::detect_and_parse(&rope.to_string()) {
                        Some(report) => {
                            let mut coverage_candidates: Vec<_> = report
                                .files
                                .iter()
                                .filter(|f| f.path == file_path)
                                .cloned()
                                .collect();
                            if coverage_candidates.is_empty() {
                                let requested_base =
                                    file_path.rsplit('/').next().unwrap_or(&file_path);
                                coverage_candidates = report
                                    .files
                                    .iter()
                                    .filter(|f| {
                                        let base = f.path.rsplit('/').next().unwrap_or(&f.path);
                                        f.path.ends_with(file_path.as_str())
                                            || file_path.ends_with(f.path.as_str())
                                            || base == requested_base
                                    })
                                    .cloned()
                                    .collect();
                                let mut unique_paths = std::collections::BTreeSet::new();
                                for f in &coverage_candidates {
                                    unique_paths.insert(f.path.clone());
                                }
                                if unique_paths.len() != 1 {
                                    coverage_candidates.clear();
                                }
                            }
                            let report = refact_codehealth::coverage::CoverageReport {
                                format: report.format,
                                files: coverage_candidates,
                            };
                            let mut complexity_by_file = HashMap::new();
                            for f in &report.files {
                                complexity_by_file.insert(f.path.clone(), health.max_complexity);
                            }
                            let churn_by_file: HashMap<String, u32> = HashMap::new();
                            let (line_pct, branch_pct, below_50) =
                                refact_codehealth::coverage_biomarkers::coverage_summary(&report);
                            msg.push_str(&format!(
                            "\nCoverage ({}): {:.0}% lines, {:.0}% branches, {} files below 50%\n",
                            report.format, line_pct, branch_pct, below_50
                        ));
                            let cov_findings =
                                refact_codehealth::coverage_biomarkers::coverage_biomarkers(
                                    &report,
                                    &complexity_by_file,
                                    &churn_by_file,
                                );
                            for f in cov_findings.iter().take(15) {
                                msg.push_str(&format!(
                                    "  {} [{:?}] {}\n",
                                    f.biomarker, f.severity, f.detail
                                ));
                            }
                        }
                        None => msg.push_str(&format!(
                            "\nCoverage file `{cov_path}` could not be parsed (unknown format).\n"
                        )),
                    }
                }
                Err(e) => msg.push_str(&format!("\nCoverage file `{cov_path}` unreadable: {e}\n")),
            }
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
            description: "Deterministic code-health for a file: per-function complexity/nesting/LOC/maintainability, clone duplication, structural + performance biomarkers, a 3-signal (defect/maintainability/performance) score with grade, and ranked refactoring targets.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("file_path", "string", "Path to the file to analyze."),
                    (
                        "coverage_file",
                        "string",
                        "Optional path to an LCOV/Cobertura/Clover coverage report to fold in.",
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
        let gcx = ccx.lock().await.app.gcx.clone();
        let dir = project_dir(gcx.clone())
            .await
            .ok_or_else(|| "no project directory available".to_string())?;
        let intel = cached_mine_history(&dir, 1000)?;
        if intel.hotspots(1).is_empty() {
            return Ok((
                false,
                tool_message(tool_call_id, "No git history found.".to_string()),
            ));
        }
        let service = gcx.codegraph.lock().await.clone();
        let assembly = build_git_risk_assembly(&intel, &dir, service.as_ref(), 15, None).await;
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
            description: "Git intelligence for the project: churn hotspots, bus factor per file, and frequently co-changed file pairs.".to_string(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeWhySource {
    kind: String,
    text: String,
    reference: String,
    priority: u8,
    order: usize,
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

async fn file_code_why_sources(
    gcx: Arc<crate::global_context::GlobalContext>,
    repo_path: &Path,
) -> Vec<CodeWhySource> {
    let mut sources = Vec::new();
    for (order, (relative, kind)) in code_why_file_kinds(repo_path).into_iter().enumerate() {
        let absolute = repo_path.join(&relative);
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
            kind: kind.to_string(),
            text,
            reference: path_ref(&relative),
            priority: code_why_priority(kind),
            order: usize::MAX / 2 + order,
        });
    }
    sources
}

fn cap_code_why_sources(mut sources: Vec<CodeWhySource>) -> Vec<CodeWhySource> {
    sources.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.order.cmp(&right.order))
            .then_with(|| left.reference.cmp(&right.reference))
    });
    sources.truncate(CODE_WHY_SOURCE_LIMIT);
    sources
}

async fn assemble_code_why_sources(
    gcx: Arc<crate::global_context::GlobalContext>,
    repo_path: &Path,
    intel: &refact_git_intel::GitIntel,
) -> Vec<CodeWhySource> {
    let mut sources = git_code_why_sources(intel);
    sources.extend(file_code_why_sources(gcx, repo_path).await);
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
        let gcx = ccx.lock().await.app.gcx.clone();
        let dir = project_dir(gcx.clone())
            .await
            .ok_or_else(|| "no project directory available".to_string())?;
        let intel = cached_mine_history(&dir, CODE_WHY_HISTORY_COMMITS)?;
        let sources = assemble_code_why_sources(gcx, &dir, &intel).await;
        let Some(msg) = code_why_output(&query, &sources, intel.commits_analyzed) else {
            return Ok((
                false,
                tool_message(
                    tool_call_id,
                    format!(
                        "No evidence-backed decisions found in commit prose, ADRs, changelogs, or merge PRs matching `{query}`."
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
            description: "Explain why the code is the way it is: mines evidence-backed decisions from significant commit prose, merge PR bodies, ADRs, and changelogs matching a query.".to_string(),
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
        let project_root = project_dir(gcx.clone()).await;
        let intel = project_root
            .as_deref()
            .and_then(|dir| cached_mine_history(dir, 1000).ok());
        let co_change = |a: &str, b: &str| -> u32 {
            co_change_count(intel.as_ref(), a, b, project_root.as_deref())
        };
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
            let file_dup_tokens: usize = file_clones.iter().map(|c| c.token_len).sum();
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
            description: "Project-wide cross-file duplication: token-level clone pairs across files, co-change-weighted DRY violations, and test smells (large/duplicated assertion blocks in test files).".to_string(),
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
        let file_path = string_arg(args, "file_path")?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let text = crate::files_in_workspace::get_file_text_from_memory_or_disk(
            gcx.clone(),
            &PathBuf::from(&file_path),
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
                    format!("Security scan for `{file_path}` found no findings (lang: {lang})."),
                ),
            ));
        }

        let mut counts: HashMap<&'static str, usize> = HashMap::new();
        for finding in &findings {
            *counts.entry(severity_label(finding.severity)).or_insert(0) += 1;
        }
        let mut msg = format!(
            "Security scan for `{}` found {} findings (lang: {}).\n",
            file_path,
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
                file_path, finding.line, finding.severity, finding.rule, finding.snippet
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
            description: "Security scan for a file: detects hardcoded secrets, SQL/command injection, dangerous eval/deserialization, TLS verification disabled, weak crypto, and insecure random usage.".to_string(),
            input_schema: json_schema_from_params(
                &[("file_path", "string", "Path to the file to scan.")],
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
        let report = service.pr_blast(&changed_files, max_depth).await?;
        let repo_dir = project_dir(gcx.clone()).await;
        let intel = repo_dir
            .as_deref()
            .and_then(|dir| cached_mine_history(dir, 1000).ok());
        let reviewers = pr_blast_suggested_reviewers(&report, repo_dir.as_deref(), intel.as_ref());

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
            description: "PR blast-radius analysis: given changed files, walks reverse codegraph dependencies to list directly and transitively impacted symbols, impacted file count, risk score, reviewer suggestions from git ownership, structural/behavioral impact classification, and index readiness. Bot authors matching [bot], agent@, or noreply are excluded.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "changed_files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Changed file paths to analyze."
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

pub struct ToolCodeMap {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolCodeMap {
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
        let files_text = service.all_files_with_text().await?;
        let cached = service.cached_graph_analytics().await?;
        let nodes = cached.data.nodes;
        let edges = cached.data.edges;
        let centrality = cached.analytics.file_centrality.truncated(5000);
        if files_text.is_empty() {
            return Ok((
                false,
                tool_message(tool_call_id, "Code graph has no indexed files.".to_string()),
            ));
        }
        let pr: HashMap<String, f64> = centrality.top_pagerank.iter().cloned().collect();
        let bt: HashMap<String, f64> = centrality.top_betweenness.iter().cloned().collect();
        let max_pr = pr.values().cloned().fold(0.0_f64, f64::max);
        let max_bt = bt.values().cloned().fold(0.0_f64, f64::max);
        let mut id_to_path: HashMap<i64, String> = HashMap::new();
        let mut symbols_by_path: HashMap<String, Vec<String>> = HashMap::new();
        for (id, name, path) in &nodes {
            id_to_path.insert(*id, path.clone());
            symbols_by_path
                .entry(path.clone())
                .or_default()
                .push(name.clone());
        }
        let mut neighbors_by_path: HashMap<String, std::collections::BTreeSet<String>> =
            HashMap::new();
        for (src, dst, _kind) in &edges {
            if let (Some(sp), Some(dp)) = (id_to_path.get(src), id_to_path.get(dst)) {
                if sp != dp {
                    neighbors_by_path
                        .entry(sp.clone())
                        .or_default()
                        .insert(dp.clone());
                }
            }
        }
        let mut scored: Vec<(String, f64)> = files_text
            .iter()
            .map(|(path, text)| {
                let symbols: Vec<refact_codewiki::selection_scoring::SymbolInfo> = symbols_by_path
                    .get(path)
                    .map(|names| {
                        names
                            .iter()
                            .map(|_| refact_codewiki::selection_scoring::SymbolInfo {
                                kind: "symbol".to_string(),
                                visibility: "public".to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let parsed = refact_codewiki::selection_scoring::ParsedFile {
                    file_info: refact_codewiki::selection_scoring::FileInfo {
                        path: path.clone(),
                        is_entry_point: refact_codewiki::entry_points::is_conventional_entry(path),
                        is_test: refact_git_intel::paths::is_test_path(path),
                        size_bytes: text.len() as u64,
                    },
                    symbols,
                };
                let score = refact_codewiki::selection_scoring::score_file(
                    &parsed,
                    pr.get(path).copied().unwrap_or(0.0),
                    bt.get(path).copied().unwrap_or(0.0),
                    max_pr,
                    max_bt,
                    false,
                    0.0,
                );
                (path.clone(), score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(60);
        let selected: std::collections::BTreeSet<String> =
            scored.iter().map(|(p, _)| p.clone()).collect();
        let pages: Vec<refact_codewiki::interlinking::GeneratedPage> = scored
            .iter()
            .map(|(path, _)| {
                let basename = path.rsplit('/').next().unwrap_or(path).to_string();
                let mut content = String::new();
                if let Some(names) = symbols_by_path.get(path) {
                    let mut names = names.clone();
                    names.sort();
                    names.dedup();
                    content.push_str(&format!("Symbols: {}\n", names.len()));
                }
                if let Some(neigh) = neighbors_by_path.get(path) {
                    content.push_str("Related files:\n");
                    for n in neigh.iter().filter(|n| selected.contains(*n)).take(15) {
                        content.push_str(&format!("- `{}`\n", n));
                    }
                }
                refact_codewiki::interlinking::GeneratedPage {
                    page_id: path.clone(),
                    title: basename,
                    page_type: "file_page".to_string(),
                    target_path: path.clone(),
                    content,
                }
            })
            .collect();
        let (forward, backlinks) =
            refact_codewiki::interlinking::attach_wiki_links_and_backlinks(&pages);
        let total_links: usize = forward.values().map(|v| v.len()).sum();
        let mut hubs: Vec<(String, usize)> = backlinks
            .iter()
            .map(|(page, links)| (page.clone(), links.len()))
            .collect();
        hubs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut msg = format!(
            "Code map: {} indexed files, {} documentation-worthy pages, {} cross-reference links.\n\nMost documentation-worthy files (selection score):\n",
            files_text.len(),
            pages.len(),
            total_links
        );
        for (path, score) in scored.iter().take(12) {
            msg.push_str(&format!("  {:.2}  {}\n", score, path));
        }
        let hub_lines: Vec<&(String, usize)> =
            hubs.iter().filter(|(_, n)| *n > 0).take(12).collect();
        if !hub_lines.is_empty() {
            msg.push_str("\nMost-referenced files (backlink hubs):\n");
            for (path, n) in hub_lines {
                msg.push_str(&format!("  {}x  {}\n", n, path));
            }
        }
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
            description: "Documentation-worthy file map: ranks files by selection score (centrality + entry-point/symbol signals) and builds a deterministic cross-reference link graph between them (forward links + backlink hubs).".to_string(),
            input_schema: json_schema_from_params(&[], &[]),
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
