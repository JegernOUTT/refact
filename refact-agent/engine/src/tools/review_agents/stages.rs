use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::global_context::GlobalContext;
use crate::tools::review_types::ReviewDepth;

pub const STAGE_KIND: &str = "review_stages";

pub const BASE_STAGE_TOOLS: &[&str] = &[
    "shell",
    "cat",
    "tree",
    "glob",
    "search_pattern",
    "search_symbol_definition",
    "search_semantic",
    "knowledge",
    "process_start",
    "process_read",
    "process_wait",
    "process_kill",
];

const STAGE_ORDER: &[&str] = &[
    "mechanical",
    "diff",
    "impact",
    "spec",
    "security",
    "dependencies",
    "simplicity",
    "concurrency",
    "tests",
    "execution",
    "browser",
    "adversarial",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StagePhase {
    Parallel,
    PostMerge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageContract {
    Findings,
    Verdicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageDepth {
    Normal,
    Deep,
    OptIn,
}

impl StageDepth {
    fn included_at(&self, depth: ReviewDepth) -> bool {
        match self {
            Self::Normal => true,
            Self::Deep => depth >= ReviewDepth::Deep,
            Self::OptIn => false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppliesWhen {
    pub always: bool,
    pub extensions: Vec<String>,
    pub path_globs: Vec<String>,
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    let path = path.replace('\\', "/");
    let pattern = pattern.trim();
    if !pattern.contains(['*', '?', '[']) {
        return path.ends_with(pattern);
    }
    match glob::Pattern::new(pattern) {
        Ok(compiled) => compiled.matches_with(
            &path,
            glob::MatchOptions {
                case_sensitive: true,
                require_literal_separator: true,
                require_literal_leading_dot: false,
            },
        ),
        Err(_) => path.ends_with(pattern),
    }
}

impl AppliesWhen {
    pub fn matches(&self, files: &[String]) -> bool {
        if self.always || (self.extensions.is_empty() && self.path_globs.is_empty()) {
            return true;
        }
        files.iter().any(|file| {
            let lowered = file.to_ascii_lowercase();
            let extension = Path::new(&lowered)
                .extension()
                .map(|ext| ext.to_string_lossy().to_string())
                .unwrap_or_default();
            self.extensions.iter().any(|candidate| {
                candidate
                    .trim_start_matches('.')
                    .eq_ignore_ascii_case(&extension)
            }) || self
                .path_globs
                .iter()
                .any(|pattern| glob_matches(&pattern.to_ascii_lowercase(), &lowered))
        })
    }
}

fn default_phase() -> StagePhase {
    StagePhase::Parallel
}

fn default_contract() -> StageContract {
    StageContract::Findings
}

fn default_depth() -> StageDepth {
    StageDepth::Normal
}

#[derive(Debug, Clone, Deserialize)]
pub struct StageSpec {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "default_phase")]
    pub phase: StagePhase,
    #[serde(default = "default_contract")]
    pub contract: StageContract,
    #[serde(default = "default_depth")]
    pub depth: StageDepth,
    #[serde(default)]
    pub writes_allowed: bool,
    #[serde(default)]
    pub applies_when: AppliesWhen,
    #[serde(default)]
    pub preferred_tools: Vec<String>,
    #[serde(default)]
    pub fallback: Option<String>,
    pub task: String,
}

impl StageSpec {
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| format!("Review: {}", self.id))
    }

    pub fn tools(&self) -> Vec<String> {
        let mut tools: Vec<String> = BASE_STAGE_TOOLS.iter().map(|t| t.to_string()).collect();
        for tool in &self.preferred_tools {
            let tool = tool.trim();
            if !tool.is_empty() && !tools.iter().any(|known| known == tool) {
                tools.push(tool.to_string());
            }
        }
        tools
    }

    pub fn order_index(&self) -> usize {
        STAGE_ORDER
            .iter()
            .position(|name| *name == self.id)
            .unwrap_or(STAGE_ORDER.len())
    }
}

fn parse_stage(filename: &str, content: &str) -> Option<StageSpec> {
    match serde_yaml::from_str::<StageSpec>(content) {
        Ok(spec) if !spec.id.trim().is_empty() && !spec.task.trim().is_empty() => Some(spec),
        Ok(_) => {
            tracing::warn!("review stage '{filename}' is missing id or task");
            None
        }
        Err(error) => {
            tracing::warn!("review stage '{filename}' is invalid: {error}");
            None
        }
    }
}

async fn overlay_from_dir(catalog: &mut BTreeMap<String, StageSpec>, dir: PathBuf) {
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let is_yaml = path
            .extension()
            .map(|ext| ext == "yaml" || ext == "yml")
            .unwrap_or(false);
        if !is_yaml {
            continue;
        }
        let Ok(content) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        if let Some(spec) = parse_stage(&filename, &content) {
            catalog.insert(spec.id.clone(), spec);
        }
    }
}

pub fn embedded_catalog() -> Vec<StageSpec> {
    let mut stages: Vec<StageSpec> =
        refact_yaml_configs::project_configs_bootstrap::embedded_defaults(STAGE_KIND)
            .into_iter()
            .filter_map(|(filename, content)| parse_stage(&filename, &content))
            .collect();
    stages.sort_by(|left, right| {
        left.order_index()
            .cmp(&right.order_index())
            .then_with(|| left.id.cmp(&right.id))
    });
    stages
}

pub async fn load_stage_catalog(gcx: Arc<GlobalContext>) -> Vec<StageSpec> {
    let mut catalog: BTreeMap<String, StageSpec> = embedded_catalog()
        .into_iter()
        .map(|spec| (spec.id.clone(), spec))
        .collect();

    let config_dir = gcx.config_dir.clone();
    overlay_from_dir(&mut catalog, config_dir.join(STAGE_KIND)).await;
    for project in crate::files_correction::get_project_dirs(gcx.clone()).await {
        overlay_from_dir(&mut catalog, project.join(".refact").join(STAGE_KIND)).await;
    }

    let mut stages: Vec<StageSpec> = catalog.into_values().collect();
    stages.sort_by(|left, right| {
        left.order_index()
            .cmp(&right.order_index())
            .then_with(|| left.id.cmp(&right.id))
    });
    stages
}

#[derive(Debug)]
pub struct StageSelection {
    pub scheduled: Vec<StageSpec>,
    pub skipped: Vec<(String, String)>,
}

pub fn select_stages(
    catalog: Vec<StageSpec>,
    depth: ReviewDepth,
    requested: Option<&[String]>,
    files: &[String],
) -> Result<StageSelection, String> {
    if let Some(requested) = requested {
        let known: Vec<&str> = catalog.iter().map(|spec| spec.id.as_str()).collect();
        let unknown: Vec<&String> = requested
            .iter()
            .filter(|name| !known.contains(&name.as_str()))
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "unknown stage(s): {}; available stages: {}",
                unknown
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                known.join(", ")
            ));
        }
    }

    let mut scheduled = Vec::new();
    let mut skipped = Vec::new();
    for spec in catalog {
        let wanted = match requested {
            Some(requested) => requested.iter().any(|name| name == &spec.id),
            None => spec.depth.included_at(depth),
        };
        if !wanted {
            let reason = match requested {
                Some(_) => "not requested".to_string(),
                None => match spec.depth {
                    StageDepth::Deep => "depth normal".to_string(),
                    StageDepth::OptIn => "opt-in stage".to_string(),
                    StageDepth::Normal => "not scheduled".to_string(),
                },
            };
            skipped.push((spec.id, reason));
            continue;
        }
        if !spec.applies_when.matches(files) {
            skipped.push((spec.id, "no matching files in scope".to_string()));
            continue;
        }
        scheduled.push(spec);
    }
    Ok(StageSelection { scheduled, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn review_stage_catalog_ships_every_stage_with_a_task_and_contract() {
        let catalog = embedded_catalog();
        let ids: Vec<&str> = catalog.iter().map(|spec| spec.id.as_str()).collect();

        for expected in STAGE_ORDER {
            assert!(ids.contains(expected), "missing stage {expected}");
        }
        for spec in &catalog {
            assert!(!spec.task.trim().is_empty(), "{} has no task", spec.id);
        }
        let adversarial = catalog
            .iter()
            .find(|spec| spec.id == "adversarial")
            .unwrap();
        assert_eq!(adversarial.phase, StagePhase::PostMerge);
        assert_eq!(adversarial.contract, StageContract::Verdicts);
        let execution = catalog.iter().find(|spec| spec.id == "execution").unwrap();
        assert!(execution.writes_allowed);
        assert!(!catalog
            .iter()
            .filter(|spec| spec.id != "execution")
            .any(|spec| spec.writes_allowed));
    }

    // search_semantic is gated on vecdb, which a test gcx has no cheap way to provide, so it is
    // exempt here; every other stage tool must resolve against the real registry.
    const VECDB_GATED_STAGE_TOOLS: &[&str] = &["search_semantic"];

    #[tokio::test]
    async fn review_stage_catalog_references_only_registered_tools() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.codegraph.lock().await = Some(std::sync::Arc::new(
            refact_codegraph::CodeGraphService::open_in_memory().unwrap(),
        ));
        let registered: HashSet<String> = crate::tools::tools_list::get_available_tools(gcx)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect();

        let mut missing = Vec::new();
        for spec in embedded_catalog() {
            for tool in spec.tools() {
                if VECDB_GATED_STAGE_TOOLS.contains(&tool.as_str()) {
                    continue;
                }
                if !registered.contains(&tool) {
                    missing.push(format!("{}: {tool}", spec.id));
                }
            }
        }

        assert!(missing.is_empty(), "unregistered stage tools: {missing:?}");
    }

    #[test]
    fn review_stage_every_stage_can_run_shell() {
        for spec in embedded_catalog() {
            assert!(
                spec.tools().contains(&"shell".to_string()),
                "{} cannot run shell",
                spec.id
            );
        }
    }

    #[test]
    fn review_stage_applies_when_skips_browser_for_a_rust_only_diff() {
        let catalog = embedded_catalog();
        let rust_only = strings(&["src/lib.rs", "src/tools/mod.rs"]);

        let selection =
            select_stages(catalog.clone(), ReviewDepth::Deep, None, &rust_only).unwrap();
        let scheduled: Vec<&str> = selection
            .scheduled
            .iter()
            .map(|spec| spec.id.as_str())
            .collect();

        assert!(!scheduled.contains(&"browser"));
        assert!(scheduled.contains(&"execution"));
        assert!(selection
            .skipped
            .iter()
            .any(|(id, reason)| id == "browser" && reason.contains("matching files")));

        let with_gui = strings(&["src/lib.rs", "gui/src/App.tsx"]);
        let selection = select_stages(catalog, ReviewDepth::Deep, None, &with_gui).unwrap();
        assert!(selection.scheduled.iter().any(|spec| spec.id == "browser"));
    }

    #[test]
    fn review_stage_depth_normal_excludes_deep_and_opt_in_stages() {
        let files = strings(&["gui/src/App.tsx"]);

        let selection =
            select_stages(embedded_catalog(), ReviewDepth::Normal, None, &files).unwrap();
        let scheduled: Vec<&str> = selection
            .scheduled
            .iter()
            .map(|spec| spec.id.as_str())
            .collect();

        assert_eq!(
            scheduled,
            [
                "mechanical",
                "diff",
                "impact",
                "spec",
                "security",
                "dependencies",
                "simplicity"
            ]
        );
        assert!(selection
            .skipped
            .iter()
            .any(|(id, reason)| id == "concurrency" && reason == "opt-in stage"));
        assert!(selection
            .skipped
            .iter()
            .any(|(id, reason)| id == "execution" && reason == "depth normal"));
    }

    #[test]
    fn review_stage_explicit_stage_list_overrides_depth_and_rejects_unknown_names() {
        let files = strings(&["src/lib.rs"]);
        let requested = strings(&["concurrency", "mechanical"]);

        let selection = select_stages(
            embedded_catalog(),
            ReviewDepth::Normal,
            Some(&requested),
            &files,
        )
        .unwrap();
        let scheduled: Vec<&str> = selection
            .scheduled
            .iter()
            .map(|spec| spec.id.as_str())
            .collect();
        assert_eq!(scheduled, ["mechanical", "concurrency"]);

        let error = select_stages(
            embedded_catalog(),
            ReviewDepth::Normal,
            Some(&strings(&["typo_stage"])),
            &files,
        )
        .unwrap_err();
        assert!(error.contains("unknown stage(s): typo_stage"));
        assert!(error.contains("available stages:"));
    }

    #[test]
    fn review_stage_glob_and_extension_matching() {
        let applies = AppliesWhen {
            always: false,
            extensions: strings(&["tsx"]),
            path_globs: strings(&["**/migrations/*.sql"]),
        };

        assert!(applies.matches(&strings(&["gui/src/App.tsx"])));
        assert!(applies.matches(&strings(&["db/migrations/001_init.sql"])));
        assert!(!applies.matches(&strings(&["src/lib.rs"])));
        assert!(AppliesWhen {
            always: true,
            ..Default::default()
        }
        .matches(&[]));
    }
}
