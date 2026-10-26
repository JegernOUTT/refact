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
    "wait",
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
    #[serde(default)]
    pub budget_minutes: Option<u64>,
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
        Ok(spec) if !spec.id.trim().is_empty() && !spec.task.trim().is_empty() => {
            if spec.budget_minutes.is_some() {
                tracing::warn!(
                    "review stage '{filename}' sets budget_minutes, which no longer has any effect: stages are stopped only after idle_timeout_secs of silence"
                );
            }
            Some(spec)
        }
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

    // search_semantic is gated on vecdb, which a test gcx has no cheap way to provide, so it is
    // exempt here; every other stage tool must resolve against the real registry.
    const VECDB_GATED_STAGE_TOOLS: &[&str] = &["search_semantic"];
}
