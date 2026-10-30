//! One bounded read projection for model tools, HTTP and the Work panel.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use super::catalog::{build_catalog, WorkCatalog};
use super::error::{WorkError, WorkFailureCause};
use super::paging::ViewBinding;
use super::privacy_guard;
use super::scope::{canonical_candidate, resolve_path, WorkCaller};
use super::types::{OverviewScope, WorkOverview, WorkOverviewRequest, WorkRootRef, WorkRow};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InspectSelector {
    Work {
        work_id: String,
    },
    Files {
        paths: Vec<String>,
    },
    Worktree {
        worktree_id: String,
    },
    ProjectWorktrees {},
    /// Read-only view of who is waiting on whom, restricted to the runs this
    /// caller may already see.
    WaitGraph {},
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectRequest {
    pub selector: InspectSelector,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub selector: InspectSelector,
    pub row: Option<WorkRow>,
    /// Visible rows that touch the same repository-relative path from a
    /// separate checkout of this project. That is an integration conflict
    /// candidate, not a physical file conflict, and never a lock.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub integration_overlaps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_graph: Option<super::deadlock::WaitGraphView>,
    /// Persisted last reader and last writer of each selected file, after
    /// destination-model filtering. Absent means nothing was recorded, not
    /// that the file is free.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_provenance: Vec<super::file_provenance::ProvenanceEntry>,
    #[serde(flatten)]
    pub page: WorkOverview,
}

pub fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

fn digest(value: &impl Serialize) -> Result<String, WorkError> {
    serde_json::to_string(value)
        .map(|v| crate::file_activity::digest_of(&v))
        .map_err(|_| WorkError::unavailable())
}

pub async fn visible_projection(
    app: &AppState,
    caller: &WorkCaller,
) -> Result<(WorkCatalog, String), WorkError> {
    let before = match &caller.model {
        Some(model) => privacy_guard::policy_key(&app.gcx, model)?,
        None => "operator".into(),
    };
    let source = caller
        .source_root
        .to_str()
        .ok_or_else(WorkError::scope_denied)?;
    let mut catalog = build_catalog(app, Some(source)).await.map_err(|_| {
        if caller.model.is_some() {
            WorkError::unavailable()
        } else {
            WorkError::projection_failure(WorkFailureCause::ScopedSourceUnavailable)
        }
    })?;
    if let Some(model) = &caller.model {
        catalog.retain_visible(privacy_guard::clear_rows(
            &app.gcx,
            catalog.all_rows(),
            model,
        )?);
        let roots = privacy_guard::clear_roots(&app.gcx, catalog.roots.all(), model)?;
        catalog.roots.by_id = roots.into_iter().map(|r| (r.id.clone(), r)).collect();
        // Coverage is deliberately fixed for the model view: a contended or
        // denied foreign source must not leak through diagnostics or counts.
        catalog.degraded.clear();
        if privacy_guard::policy_key(&app.gcx, model)? != before {
            return Err(WorkError::policy_unavailable());
        }
    }
    Ok((catalog, before))
}

fn row_path(row: &WorkRow, raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(row.root.path.as_deref()?).join(path)
    };
    canonical_candidate(&path).ok()
}

fn path_matches(row: &WorkRow, selected: &[PathBuf]) -> bool {
    row.declared_files
        .iter()
        .chain(row.observed_files.iter())
        .chain(row.metadata.observed_reads.iter())
        .filter_map(|path| row_path(row, path))
        .any(|path| {
            selected
                .iter()
                .any(|selected| path == *selected || path.starts_with(selected))
        })
}

/// Repository-relative form of one row path: relative declarations are taken
/// as-is, absolute observations are stripped of the row's own checkout root.
fn row_relative_path(row: &WorkRow, raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return None;
    }
    if !path.is_absolute() {
        return Some(path.to_path_buf());
    }
    let root = canonical_candidate(Path::new(row.root.path.as_deref()?)).ok()?;
    canonical_candidate(path)
        .ok()?
        .strip_prefix(&root)
        .ok()
        .map(Path::to_path_buf)
}

/// Same repository-relative path from a different checkout of the project.
fn logical_path_matches(row: &WorkRow, caller: &WorkCaller, selected_relative: &[PathBuf]) -> bool {
    let Some(root) = row.root.path.as_deref() else {
        return false;
    };
    let same_checkout = canonical_candidate(Path::new(root))
        .ok()
        .zip(canonical_candidate(&caller.checkout_root).ok())
        .is_some_and(|(row_root, caller_root)| row_root == caller_root);
    if same_checkout {
        return false;
    }
    row.declared_files
        .iter()
        .chain(row.observed_files.iter())
        .chain(row.metadata.observed_reads.iter())
        .filter_map(|path| row_relative_path(row, path))
        .any(|path| {
            selected_relative
                .iter()
                .any(|selected| path == *selected || path.starts_with(selected))
        })
}

fn caller_relative_paths(caller: &WorkCaller, selected: &[PathBuf]) -> Vec<PathBuf> {
    let roots: Vec<PathBuf> = [&caller.checkout_root, &caller.source_root]
        .into_iter()
        .filter_map(|root| canonical_candidate(root).ok())
        .collect();
    selected
        .iter()
        .filter_map(|path| {
            roots
                .iter()
                .find_map(|root| path.strip_prefix(root).ok().map(Path::to_path_buf))
        })
        .collect()
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct WorkSnapshot {
    pub rows: Vec<WorkRow>,
    pub roots: Vec<WorkRootRef>,
    pub detail: Option<WorkRow>,
    pub degraded: Vec<String>,
    pub advisory: BTreeSet<String>,
    pub file_provenance: Vec<super::file_provenance::ProvenanceEntry>,
    pub wait_graph: Option<super::deadlock::WaitGraphView>,
    pub selected_paths: Vec<PathBuf>,
    pub guard_rows: Vec<WorkRow>,
}

impl WorkSnapshot {
    pub fn total(&self) -> usize {
        if self.rows.is_empty() {
            self.roots.len()
        } else {
            self.rows.len()
        }
    }

    pub fn units(&self) -> usize {
        self.rows
            .len()
            .saturating_add(self.roots.len())
            .saturating_add(usize::from(self.detail.is_some()))
            .saturating_add(self.advisory.len())
            .saturating_add(self.selected_paths.len())
            .saturating_add(self.file_provenance.len())
            .saturating_add(self.guard_rows.len())
            .saturating_add(
                self.wait_graph
                    .as_ref()
                    .map_or(0, |g| g.nodes.len().saturating_add(g.edges.len())),
            )
            .saturating_add(
                self.rows
                    .iter()
                    .chain(self.detail.iter())
                    .chain(self.guard_rows.iter())
                    .fold(0usize, |n, row| n.saturating_add(row.source_records.len())),
            )
    }

    pub fn signature(&self, binding: &ViewBinding) -> Result<(String, usize), WorkError> {
        use sha2::{Digest, Sha256};
        struct Counter {
            bytes: usize,
            hash: Sha256,
        }
        impl std::io::Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.bytes = self
                    .bytes
                    .checked_add(bytes.len())
                    .ok_or_else(|| std::io::Error::other("snapshot size overflow"))?;
                self.hash.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = Counter {
            bytes: 0,
            hash: Sha256::new(),
        };
        serde_json::to_writer(&mut output, &(binding, self)).map_err(|_| {
            WorkError::projection_failure(WorkFailureCause::SnapshotEncodingUnavailable)
        })?;
        for row in self
            .rows
            .iter()
            .chain(self.detail.iter())
            .chain(self.guard_rows.iter())
        {
            serde_json::to_writer(
                &mut output,
                &(
                    &row.source_records,
                    row.provenance_complete,
                    row.metadata.source_revision,
                ),
            )
            .map_err(|_| {
                WorkError::projection_failure(WorkFailureCause::SnapshotEncodingUnavailable)
            })?;
        }
        Ok((hex::encode(output.hash.finalize()), output.bytes))
    }

    pub fn slice(&self, offset: usize, end: usize) -> Self {
        let rows = self
            .rows
            .iter()
            .skip(offset)
            .take(end - offset)
            .cloned()
            .collect::<Vec<_>>();
        let used: HashSet<_> = rows.iter().map(|r| &r.root.id).collect();
        let roots = if self.rows.is_empty() {
            self.roots
                .iter()
                .skip(offset)
                .take(end - offset)
                .cloned()
                .collect()
        } else {
            self.roots
                .iter()
                .filter(|r| used.contains(&r.id))
                .cloned()
                .collect()
        };
        Self {
            advisory: self
                .advisory
                .iter()
                .filter(|key| rows.iter().any(|r| r.key.to_string() == **key))
                .cloned()
                .collect(),
            rows,
            roots,
            detail: self.detail.clone(),
            degraded: self.degraded.clone(),
            file_provenance: self.file_provenance.clone(),
            wait_graph: self.wait_graph.clone(),
            selected_paths: self.selected_paths.clone(),
            guard_rows: self.guard_rows.clone(),
        }
    }
}

fn row_scope(row: &WorkRow) -> Result<String, WorkError> {
    digest(&(
        &row.key,
        row.kind,
        &row.chat_id,
        &row.parent_chat_id,
        &row.root_chat_id,
        &row.metadata.run,
        &row.metadata.task_id,
        &row.metadata.card_id,
        &row.root.id,
        &row.root.project,
        &row.root.path,
        &row.root.source_path,
    ))
}

fn revalidate_snapshot(
    app: &AppState,
    caller: &WorkCaller,
    disclosure: &str,
    catalog: &WorkCatalog,
    provenance: &[super::file_provenance::ProvenanceEntry],
    snapshot: &WorkSnapshot,
) -> Result<(), WorkError> {
    for row in snapshot
        .rows
        .iter()
        .chain(snapshot.detail.iter())
        .chain(snapshot.guard_rows.iter())
    {
        let current = catalog
            .get(&row.key.to_string())
            .ok_or_else(WorkError::cursor_expired)?;
        if row_scope(row)? != row_scope(current)?
            || current.metadata.source_revision < row.metadata.source_revision
            || !row
                .source_records
                .iter()
                .all(|record| current.source_records.contains(record))
        {
            return Err(WorkError::cursor_expired());
        }
    }
    for root in &snapshot.roots {
        let current = catalog
            .roots
            .by_id
            .get(&root.id)
            .ok_or_else(WorkError::cursor_expired)?;
        if root.project != current.project
            || root.path != current.path
            || root.source_path != current.source_path
        {
            return Err(WorkError::cursor_expired());
        }
    }
    if !snapshot
        .file_provenance
        .iter()
        .all(|entry| provenance.contains(entry))
    {
        return Err(WorkError::cursor_expired());
    }
    for path in &snapshot.selected_paths {
        if resolve_path(path.to_str().ok_or_else(WorkError::scope_denied)?, caller)? != *path {
            return Err(WorkError::scope_denied());
        }
    }
    if let Some(model) = &caller.model {
        if privacy_guard::policy_key(&app.gcx, model)? != disclosure {
            return Err(WorkError::policy_unavailable());
        }
        let mut audit = snapshot
            .rows
            .iter()
            .chain(snapshot.detail.iter())
            .chain(snapshot.guard_rows.iter())
            .cloned()
            .collect::<Vec<_>>();
        for row in &mut audit {
            row.observed_files
                .extend(row.source_records.iter().map(|r| r.path.clone()));
        }
        let expected = audit.len();
        if privacy_guard::clear_rows(&app.gcx, audit, model)?.len() != expected
            || privacy_guard::clear_roots(&app.gcx, snapshot.roots.clone(), model)?.len()
                != snapshot.roots.len()
            || super::file_provenance::clear_for_model(
                &app.gcx,
                Some(model),
                snapshot.file_provenance.clone(),
            )?
            .len()
                != snapshot.file_provenance.len()
        {
            return Err(WorkError::cursor_expired());
        }
        let selected = snapshot
            .selected_paths
            .iter()
            .map(|path| WorkRootRef {
                id: String::new(),
                kind: super::types::WorkRootKind::Project,
                path: Some(path.to_string_lossy().into_owned()),
                branch: None,
                project: catalog.project.clone(),
                source_path: None,
                presence: Default::default(),
            })
            .collect();
        if privacy_guard::clear_roots(&app.gcx, selected, model)?.len()
            != snapshot.selected_paths.len()
        {
            return Err(WorkError::not_visible());
        }
        if privacy_guard::policy_key(&app.gcx, model)? != disclosure {
            return Err(WorkError::policy_unavailable());
        }
    }
    Ok(())
}

fn page(
    app: &AppState,
    caller: &WorkCaller,
    disclosure: String,
    request: &WorkOverviewRequest,
    mut snapshot: WorkSnapshot,
    filter: String,
    catalog: &WorkCatalog,
) -> Result<(WorkOverview, WorkSnapshot), WorkError> {
    let settings = crate::file_activity::settings();
    snapshot
        .rows
        .sort_by_key(|row| (std::cmp::Reverse(row.updated_at_ms), row.key.to_string()));
    snapshot.roots.sort_by(|a, b| a.id.cmp(&b.id));
    let binding = ViewBinding {
        project: super::roots::project_identity(&caller.source_root),
        principal: caller.principal.clone(),
        disclosure: disclosure.clone(),
        filter,
    };
    let provenance = snapshot.file_provenance.clone();
    let frozen = app.work_views.frozen_page(
        binding,
        snapshot,
        request.rows.unwrap_or(settings.overview_default_rows),
        request.cursor.as_deref(),
        &settings,
        now_ms(),
        |saved| revalidate_snapshot(app, caller, &disclosure, catalog, &provenance, saved),
    )?;
    let window = frozen.window;
    let snapshot = frozen.snapshot;
    let page_unit = if snapshot.rows.is_empty() && !snapshot.roots.is_empty() {
        "worktrees"
    } else {
        "work"
    };
    let mut records = Vec::new();
    if caller.model.is_some() {
        let paths = snapshot.selected_paths.iter().cloned().chain(
            snapshot
                .file_provenance
                .iter()
                .map(|entry| PathBuf::from(&entry.path)),
        );
        records = crate::privacy::records::declared_file_records(&app.gcx, paths)
            .map_err(|_| WorkError::policy_unavailable())?;
        records.extend(
            snapshot
                .guard_rows
                .iter()
                .flat_map(|row| row.source_records.clone()),
        );
        if privacy_guard::policy_key(
            &app.gcx,
            caller
                .model
                .as_ref()
                .ok_or_else(WorkError::policy_unavailable)?,
        )? != disclosure
        {
            return Err(WorkError::policy_unavailable());
        }
    }
    Ok((
        WorkOverview {
            snapshot_id: window.snapshot_id,
            seq: window.seq,
            project: super::roots::project_identity(&caller.source_root),
            coverage: "partial".into(),
            page_unit: page_unit.into(),
            scope: request.scope,
            root: request.root.clone(),
            generated_at_ms: now_ms(),
            snapshot_observed_at_ms: frozen.observed_at,
            snapshot_stale: frozen.stale,
            rows: snapshot.rows.clone(),
            total: frozen.total,
            returned: window.end - window.offset,
            truncated: window.next_cursor.is_some(),
            next_cursor: window.next_cursor,
            roots: snapshot.roots.clone(),
            withheld_rows: 0,
            degraded: snapshot.degraded.clone(),
            source_records: records,
        },
        snapshot,
    ))
}

pub async fn overview(
    app: &AppState,
    caller: &WorkCaller,
    mut request: WorkOverviewRequest,
) -> Result<WorkOverview, WorkError> {
    if request.scope == OverviewScope::Root {
        request.root =
            Some(caller.root_chat_id.clone().ok_or_else(|| {
                WorkError::invalid("root scope requires an existing caller chat")
            })?);
    } else if request.root.is_some() {
        return Err(WorkError::invalid("root is not a project-scope filter"));
    }
    let selected = request
        .path
        .as_ref()
        .map(|path| resolve_path(path, caller))
        .transpose()?;
    let (catalog, disclosure) = visible_projection(app, caller).await?;
    let settings = crate::file_activity::settings();
    let recent = now_ms().saturating_sub(settings.recent_window_secs.saturating_mul(1000));
    let rows: Vec<_> = catalog
        .all_rows()
        .into_iter()
        .filter(|row| {
            request.root.as_ref().is_none_or(|root| {
                row.root_chat_id.as_ref() == Some(root) || row.chat_id.as_ref() == Some(root)
            })
        })
        .filter(|row| match request.states.as_ref() {
            Some(states) => states.contains(&row.state),
            None => !row.state.is_terminal() || row.updated_at_ms >= recent,
        })
        .filter(|row| {
            selected
                .as_ref()
                .is_none_or(|path| path_matches(row, std::slice::from_ref(path)))
        })
        .collect();
    let used: HashSet<_> = rows.iter().map(|r| &r.root.id).collect();
    let roots = catalog
        .roots
        .by_id
        .values()
        .filter(|r| used.contains(&r.id))
        .cloned()
        .collect();
    let mut filter = request.clone();
    filter.cursor = None;
    page(
        app,
        caller,
        disclosure,
        &request,
        WorkSnapshot {
            rows,
            roots,
            selected_paths: selected.into_iter().collect(),
            degraded: catalog.degraded.clone(),
            ..Default::default()
        },
        digest(&(&filter, &request.root, &caller.checkout_root))?,
        &catalog,
    )
    .map(|(page, _)| page)
}

pub async fn inspect(
    app: &AppState,
    caller: &WorkCaller,
    request: InspectRequest,
) -> Result<Inspection, WorkError> {
    let paths = match &request.selector {
        InspectSelector::Files { paths } => {
            if paths.is_empty() || paths.len() > crate::file_activity::settings().inspect_max_paths
            {
                return Err(WorkError::invalid(
                    "paths must be a nonempty list within the configured limit",
                ));
            }
            paths
                .iter()
                .map(|path| resolve_path(path, caller))
                .collect::<Result<Vec<_>, _>>()?
        }
        _ => Vec::new(),
    };
    let (catalog, disclosure) = visible_projection(app, caller).await?;
    let mut row = None;
    let mut roots = Vec::new();
    let mut advisory = HashSet::new();
    let rows = match &request.selector {
        InspectSelector::Work { work_id } => {
            let found = catalog
                .find(work_id)
                .cloned()
                .ok_or_else(WorkError::not_visible)?;
            let children = catalog
                .all_rows()
                .into_iter()
                .filter(|r| r.parent_chat_id.is_some() && r.parent_chat_id == found.chat_id)
                .collect();
            row = Some(found);
            children
        }
        InspectSelector::Files { .. } => {
            let relative = caller_relative_paths(caller, &paths);
            catalog
                .all_rows()
                .into_iter()
                .filter(|r| {
                    if path_matches(r, &paths) {
                        return true;
                    }
                    if logical_path_matches(r, caller, &relative) {
                        advisory.insert(r.key.to_string());
                        return true;
                    }
                    false
                })
                .collect()
        }
        InspectSelector::Worktree { worktree_id } => {
            let root = catalog
                .roots
                .by_id
                .get(worktree_id)
                .cloned()
                .ok_or_else(WorkError::not_visible)?;
            roots.push(root);
            catalog
                .all_rows()
                .into_iter()
                .filter(|r| r.root.id == *worktree_id)
                .collect()
        }
        InspectSelector::ProjectWorktrees {} => {
            roots = catalog.roots.by_id.values().cloned().collect();
            Vec::new()
        }
        InspectSelector::WaitGraph {} => Vec::new(),
    };
    let mut wait_graph = match &request.selector {
        InspectSelector::WaitGraph {} => Some(super::deadlock::visible_view(
            app,
            &catalog
                .all_rows()
                .iter()
                .map(|row| row.key.to_string())
                .collect(),
        )),
        _ => None,
    };
    let guard_rows = if let Some(graph) = &mut wait_graph {
        if caller.model.is_some() {
            graph.withheld_nodes = 0;
        }
        graph.nodes.sort_by_key(|node| node.run.to_string());
        catalog
            .all_rows()
            .into_iter()
            .filter(|row| graph.nodes.iter().any(|node| node.run.work == row.key))
            .collect()
    } else {
        Vec::new()
    };
    let overview_request = WorkOverviewRequest {
        cursor: request.cursor.clone(),
        rows: request.limit,
        ..Default::default()
    };
    let filter = digest(&(&request.selector, request.limit, &caller.checkout_root))?;
    let file_keys: Vec<String> = paths
        .iter()
        .map(|path| crate::file_activity::resolve_key(path).0)
        .collect();
    let file_provenance = super::file_provenance::clear_for_model(
        &app.gcx,
        caller.model.as_ref(),
        super::file_provenance::entries_for_files(&file_keys).await,
    )?;
    let (page, snapshot) = page(
        app,
        caller,
        disclosure,
        &overview_request,
        WorkSnapshot {
            rows,
            roots,
            detail: row,
            degraded: catalog.degraded.clone(),
            advisory: advisory.into_iter().collect(),
            wait_graph,
            file_provenance,
            selected_paths: paths,
            guard_rows,
        },
        filter,
        &catalog,
    )?;
    Ok(Inspection {
        selector: request.selector,
        row: snapshot.detail,
        integration_overlaps: snapshot.advisory.into_iter().collect(),
        wait_graph: snapshot.wait_graph,
        file_provenance: snapshot.file_provenance,
        page,
    })
}

pub fn attach_provenance(
    app: &AppState,
    message: &mut crate::call_validation::ChatMessage,
    page: &WorkOverview,
    detail: Option<&WorkRow>,
) -> Result<(), WorkError> {
    let records = page
        .rows
        .iter()
        .chain(detail)
        .flat_map(|row| row.source_records.clone());
    let paths = page
        .roots
        .iter()
        .flat_map(|root| root.path.iter().chain(root.source_path.iter()))
        .map(PathBuf::from);
    let roots = crate::privacy::records::declared_file_records(&app.gcx, paths)
        .map_err(|_| WorkError::policy_unavailable())?;
    crate::privacy::records::merge_records(
        message,
        records.chain(roots).chain(page.source_records.clone()),
    );
    Ok(())
}
