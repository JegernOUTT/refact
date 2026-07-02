use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};

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

fn string_arg(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) => Err(format!("argument `{key}` is not a string: {v:?}")),
        None => Err(format!("argument `{key}` is missing")),
    }
}

async fn project_dir(gcx: Arc<crate::global_context::GlobalContext>) -> Option<PathBuf> {
    crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
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
        let overview = service.overview(15).await?;
        let mut msg = format!(
            "Code graph overview:\n  nodes: {}\n  edges: {}\n  connected components: {}\n  strongly-connected components: {} (largest {})\n\nMost central symbols (PageRank):\n",
            overview.node_count,
            overview.edge_count,
            overview.component_count,
            overview.scc_count,
            overview.largest_scc
        );
        for (name, score) in &overview.top_pagerank {
            msg.push_str(&format!("  {:.4}  {}\n", score, name));
        }
        msg.push_str("\nKey connectors (betweenness centrality):\n");
        for (name, score) in overview.top_betweenness.iter().filter(|(_, s)| *s > 0.0) {
            msg.push_str(&format!("  {:.2}  {}\n", score, name));
        }
        if let Ok(mut communities) = service.communities().await {
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
        if let Ok(mut dead) = service.dead_code().await {
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
        }
        let all_files = service.all_files_with_text().await.unwrap_or_default();
        if let Ok(centrality) = service.per_file_centrality(100).await {
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
            if let (Ok(nodes), Ok(edges)) =
                (service.graph_nodes().await, service.graph_edges().await)
            {
                let graph = refact_codewiki::graph_intelligence::CodeGraph {
                    nodes: nodes
                        .iter()
                        .map(
                            |(id, _name, path)| refact_codewiki::graph_intelligence::GraphNode {
                                id: id.to_string(),
                                node_type: "symbol".to_string(),
                                file_path: path.clone(),
                            },
                        )
                        .collect(),
                    edges: edges
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
                let stored_path = if nodes.iter().any(|(_, _, p)| p == &file_path) {
                    Some(file_path.clone())
                } else {
                    let mut candidates: std::collections::BTreeSet<&str> =
                        std::collections::BTreeSet::new();
                    let requested_base = file_path.rsplit('/').next().unwrap_or(&file_path);
                    for (_, _, p) in &nodes {
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
        let intel = refact_git_intel::mine_history(&dir, 1000)?;
        let hotspots = intel.hotspots(15);
        if hotspots.is_empty() {
            return Ok((
                false,
                tool_message(tool_call_id, "No git history found.".to_string()),
            ));
        }
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let temporal = intel.temporal_hotspots(now_ts, 15);
        let mut msg = format!(
            "Git risk (over {} commits) — recency-weighted hotspots:\n",
            intel.commits_analyzed
        );
        let temporal_map: std::collections::HashMap<&String, f64> =
            temporal.iter().map(|(p, s)| (p, *s)).collect();
        for h in &hotspots {
            let bus = intel.bus_factor(&h.path);
            let mut flags = Vec::new();
            if intel.ownership_risk(&h.path) {
                flags.push("ownership-risk");
            }
            if intel.knowledge_loss(&h.path) {
                flags.push("knowledge-loss");
            }
            let hot = temporal_map.get(&h.path).copied().unwrap_or(0.0);
            msg.push_str(&format!(
                "  churn={} temporal={:.2} bus_factor={} {}{}\n",
                h.churn,
                hot,
                bus,
                h.path,
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
        let coupling = refact_git_intel::coupling::build_coupling_graph(&intel, 8);
        if !coupling.edges.is_empty() {
            msg.push_str("\nStrongest coupling (normalized):\n");
            for e in coupling.edges.iter().take(8) {
                msg.push_str(&format!(
                    "  strength={:.2} ({}x)  {} <-> {}\n",
                    e.strength, e.co_changes, e.a, e.b
                ));
            }
        }
        let hotspot_paths: Vec<String> = hotspots.iter().map(|h| h.path.clone()).collect();
        let reviewers = refact_git_intel::coupling::reviewer_suggestions(&intel, &hotspot_paths, 5);
        if !reviewers.is_empty() {
            msg.push_str(
                "\nSuggested reviewers (by ownership of hot files + co-change neighbors):\n",
            );
            for (author, score) in &reviewers {
                msg.push_str(&format!("  {} (score {:.2})\n", author, score));
            }
        }
        let entropy_map = intel.change_entropy();
        let mut biomarker_lines: Vec<String> = Vec::new();
        for h in hotspots.iter().take(10) {
            let p = &h.path;
            let nloc = match crate::files_in_workspace::get_file_text_from_memory_or_disk(
                gcx.clone(),
                &dir.join(p),
            )
            .await
            {
                Ok(rope) => rope
                    .to_string()
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .count() as u32,
                Err(_) => 0,
            };
            let (added_90d, deleted_90d) = intel.lines_in_window(p, now_ts, 90);
            let (primary_owner_name, primary_owner_commit_pct) = intel.primary_owner(p);
            let (recent_owner_name, recent_owner_commit_pct) = intel.recent_owner(p, now_ts, 90);
            let meta = refact_codehealth::git_biomarkers::GitMeta {
                file_path: p.clone(),
                change_entropy: entropy_map.get(p).copied().unwrap_or(0.0),
                change_entropy_pct: intel.change_entropy_pct(p),
                commit_count_90d: intel.commit_count_in_window(p, now_ts, 90),
                commit_count_total: intel.file_churn.get(p).copied().unwrap_or(0),
                is_hotspot: true,
                is_stable: false,
                churn_percentile: intel.churn_percentile(p),
                lines_added_90d: added_90d,
                lines_deleted_90d: deleted_90d,
                nloc,
                contributor_count: intel
                    .file_authors
                    .get(p)
                    .map(|m| m.len() as u32)
                    .unwrap_or(0),
                primary_owner_commit_pct,
                primary_owner_name,
                recent_owner_name,
                recent_owner_commit_pct,
                bus_factor: intel.bus_factor(p) as u32,
                prior_defect_count: 0,
                repo_active_contributors_90d: Some(intel.active_contributors_in_window(now_ts, 90)),
                repo_function_mod_p80: None,
                co_change_partners: intel
                    .co_change_partners(p, 1)
                    .into_iter()
                    .map(
                        |(path, c)| refact_codehealth::git_biomarkers::CoChangePartner {
                            path,
                            co_change_count: c as f64,
                        },
                    )
                    .collect(),
                top_authors: intel
                    .ownership(p)
                    .into_iter()
                    .map(|o| (o.author, o.commits))
                    .collect(),
                functions: Vec::new(),
                repo_commit_counts: intel.file_churn.clone(),
                import_edges: std::collections::HashSet::new(),
            };
            for f in refact_codehealth::git_biomarkers::git_biomarkers(&meta) {
                if f.biomarker == "hidden_coupling" {
                    continue;
                }
                biomarker_lines.push(format!(
                    "  {} [{:?}] {} — {}",
                    f.biomarker, f.severity, p, f.detail
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
        Ok((false, tool_message(tool_call_id, msg)))
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
}

pub struct ToolCodeWhy {
    pub config_path: String,
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
        let dir = project_dir(gcx)
            .await
            .ok_or_else(|| "no project directory available".to_string())?;
        let messages = refact_git_intel::collect_commit_messages(&dir, 2000)?;
        let sources: Vec<refact_codewiki::DecisionSource> = messages
            .into_iter()
            .map(|text| refact_codewiki::DecisionSource {
                kind: "commit".to_string(),
                text,
            })
            .collect();
        let decisions = refact_codewiki::sources::extract_all(&sources);
        let mut corr_by_statement: HashMap<String, u32> = HashMap::new();
        for d in &decisions {
            *corr_by_statement
                .entry(refact_codewiki::gate::normalize_text(&d.statement))
                .or_insert(0) += 1;
        }
        let needle = query.to_lowercase();
        let mut matched: Vec<_> = decisions
            .into_iter()
            .filter(|d| d.statement.to_lowercase().contains(&needle))
            .collect();
        matched.sort_by(|a, b| b.source_rank.cmp(&a.source_rank));
        let mut deduped: Vec<refact_codewiki::sources::ExtractedDecision> = Vec::new();
        for d in matched {
            let mut hits: Vec<refact_codewiki::semantic_match::SearchHit> = deduped
                .iter()
                .enumerate()
                .map(|(i, k)| refact_codewiki::semantic_match::SearchHit {
                    page_id: refact_codewiki::semantic_match::decision_page_id(&i.to_string()),
                    score: refact_codewiki::gate::decision_similarity(&d.statement, &k.statement),
                })
                .collect();
            hits.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            match refact_codewiki::semantic_match::find_duplicate_decision(
                &hits,
                refact_codewiki::semantic_match::DEFAULT_DEDUP_TAU,
                &std::collections::HashSet::new(),
            ) {
                Some(_) => {}
                None => {
                    deduped.push(d);
                }
            }
        }
        let mut matched = deduped;
        matched.truncate(20);
        if matched.is_empty() {
            return Ok((
                false,
                tool_message(
                    tool_call_id,
                    format!(
                        "No evidence-backed decisions found in commit history matching `{query}`."
                    ),
                ),
            ));
        }
        let mut msg = format!("Decisions matching `{query}` (evidence-ranked, grounded):\n");
        for d in &matched {
            let verdict = refact_codewiki::gate::verify_quote(&d.statement, &d.evidence, 0.6);
            let verification = match verdict {
                refact_codewiki::gate::Verdict::Exact => refact_codewiki::gate::Verification::Exact,
                refact_codewiki::gate::Verdict::Fuzzy => refact_codewiki::gate::Verification::Fuzzy,
                refact_codewiki::gate::Verdict::Unverified => {
                    refact_codewiki::gate::Verification::Unverified
                }
            };
            let corr = corr_by_statement
                .get(&refact_codewiki::gate::normalize_text(&d.statement))
                .copied()
                .unwrap_or(1);
            let confidence =
                refact_codewiki::gate::compute_confidence(d.source_rank, corr, verification);
            let grounded = match verdict {
                refact_codewiki::gate::Verdict::Exact => "grounded",
                refact_codewiki::gate::Verdict::Fuzzy => "partial",
                refact_codewiki::gate::Verdict::Unverified => "ungrounded",
            };
            msg.push_str(&format!(
                "  [{:?} rank={} src={} conf={:.2} {}] {}\n",
                d.status, d.source_rank, d.source_kind, confidence, grounded, d.statement
            ));
        }
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
            description: "Explain why the code is the way it is: mines evidence-backed decisions from commit history matching a query.".to_string(),
            input_schema: json_schema_from_params(
                &[("query", "string", "Topic or keyword to find decisions about.")],
                &["query"],
            ),
            output_schema: None,
            annotations: None,
        }
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
        let service = gcx
            .codegraph
            .lock()
            .await
            .clone()
            .ok_or_else(|| "codegraph is not available".to_string())?;
        let files_text = service.all_files_with_text().await?;
        let triples: Vec<(String, String, String)> = files_text
            .iter()
            .map(|(p, t)| {
                (
                    p.clone(),
                    refact_codegraph::lang_from_path(p).to_string(),
                    t.clone(),
                )
            })
            .collect();
        let clones = refact_codehealth::duplication::detect_cross_file_clones(&triples);
        if clones.is_empty() {
            return Ok((
                false,
                tool_message(tool_call_id, "No cross-file clones detected.".to_string()),
            ));
        }
        let dup_pct = refact_codehealth::duplication::cross_file_duplication_pct(&triples) * 100.0;
        let intel = project_dir(gcx.clone())
            .await
            .and_then(|dir| refact_git_intel::mine_history(&dir, 1000).ok());
        let co_change = |a: &str, b: &str| -> u32 {
            match &intel {
                Some(i) => i
                    .co_change
                    .get(&(a.to_string(), b.to_string()))
                    .or_else(|| i.co_change.get(&(b.to_string(), a.to_string())))
                    .copied()
                    .unwrap_or(0),
                None => 0,
            }
        };
        let mut msg = format!(
            "Cross-file duplication: {:.1}% of tokens are in cross-file clones ({} pairs).\n",
            dup_pct,
            clones.len()
        );
        for c in clones.iter().take(15) {
            msg.push_str(&format!(
                "  {} tokens: {}:{} <-> {}:{}\n",
                c.token_len, c.file_a, c.line_a, c.file_b, c.line_b
            ));
        }
        let text_by_path: HashMap<String, (String, String)> = files_text
            .into_iter()
            .map(|(p, t)| {
                let lang = refact_codegraph::lang_from_path(&p).to_string();
                (p, (lang, t))
            })
            .collect();
        let tokens_by_path: HashMap<String, usize> = text_by_path
            .iter()
            .map(|(p, (lang, text))| {
                (
                    p.clone(),
                    refact_codehealth::duplication::tokenize(lang, text).len(),
                )
            })
            .collect();
        let mut per_file: HashMap<
            String,
            Vec<&refact_codehealth::duplication::CrossFileClonePair>,
        > = HashMap::new();
        for c in &clones {
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
            let file_total = tokens_by_path.get(path).copied().unwrap_or(0).max(1);
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
                if let Some((lang, text)) = text_by_path.get(path) {
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
        let nodes = service.graph_nodes().await?;
        let edges = service.graph_edges().await?;
        let centrality = service.per_file_centrality(5000).await?;
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
}
