use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::desk_actions::DeskAction;

pub const MAX_BATCH_WORK_IDS: usize = 100;
pub const INVALID_BATCH_IDS: &str = "ids must contain between 1 and 100 non-empty values";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkActionsBatchQuery {
    pub project_id: Option<String>,
    pub ids: Option<String>,
}

#[derive(Serialize)]
pub struct WorkDeskActions {
    pub work_id: String,
    pub actions: Vec<DeskAction>,
    pub error: Option<String>,
}

pub fn parse_work_ids(raw: Option<&str>) -> Result<Vec<String>, &'static str> {
    let raw = raw.ok_or(INVALID_BATCH_IDS)?;
    let ids: Vec<_> = raw.split(',').collect();
    if ids.is_empty() || ids.len() > MAX_BATCH_WORK_IDS || ids.iter().any(|id| id.is_empty()) {
        return Err(INVALID_BATCH_IDS);
    }
    let mut seen = HashSet::with_capacity(ids.len());
    Ok(ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
#[path = "desk_actions_batch_tests.rs"]
mod tests;
