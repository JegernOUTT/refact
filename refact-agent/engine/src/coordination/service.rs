//! One bounded read projection for model tools, HTTP and the Work panel.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use super::catalog::{build_catalog_for_destination, WorkCatalog};
use super::error::{WorkError, WorkFailureCause};
use super::paging::ViewBinding;
use super::privacy_guard;
use super::scope::{canonical_candidate, resolve_path, WorkCaller};
use super::types::{OverviewScope, WorkOverview, WorkOverviewRequest, WorkRootRef, WorkRow};

static WORK_PROJECTION_PERMITS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

pub(super) async fn projection_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, WorkError> + Send + 'static,
) -> Result<T, WorkError> {
    use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
    let wait = perf_diagnostics::span(PerfComponent::WorkContextCapacityWait, None, None);
    let acquired = WORK_PROJECTION_PERMITS.acquire().await;
    wait.finish(
        if acquired.is_ok() {
            PerfOutcome::Success
        } else {
            PerfOutcome::Failure
        },
        None,
        None,
        None,
        None,
    );
    let permit = acquired.map_err(|_| WorkError::unavailable())?;
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| WorkError::unavailable())?;
    #[cfg(test)]
    if let Ok(Some((entered, release))) =
        PROJECTION_POST_WORK_GATE.try_with(|gate| gate.borrow_mut().take())
    {
        let _ = entered.send(());
        let _ = release.await;
    }
    result
}

#[cfg(test)]
tokio::task_local! {
    pub(super) static PROJECTION_POST_WORK_GATE: std::cell::RefCell<Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>>;
}

fn check_disclosure(
    app: &AppState,
    caller: &WorkCaller,
    disclosure: &str,
) -> Result<(), WorkError> {
    if let Some(model) = &caller.model {
        if privacy_guard::policy_key(&app.gcx, model)? != disclosure {
            return Err(WorkError::policy_unavailable());
        }
    }
    Ok(())
}

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

fn encoding_failure(cause: serde_json::Error) -> WorkError {
    let error = WorkError::projection_failure(WorkFailureCause::SnapshotEncodingUnavailable);
    tracing::debug!(request_id = %error.request_id, internal_cause = %cause, "work encoding failure");
    error
}

const PROVENANCE_RECORD_CAP: usize = 256;
const PROVENANCE_BYTE_CAP: usize = 64 * 1024 - 64;

struct ProvenanceBudget {
    records: Vec<refact_privacy::FileRecord>,
    seen: HashSet<refact_privacy::FileRecord>,
    bytes: usize,
}

impl ProvenanceBudget {
    fn new() -> Self {
        Self {
            records: Vec::new(),
            seen: HashSet::new(),
            bytes: 2,
        }
    }

    fn extend(
        &mut self,
        records: impl IntoIterator<Item = refact_privacy::FileRecord>,
    ) -> Result<(), WorkError> {
        for record in records {
            if self.seen.contains(&record) {
                continue;
            }
            let bytes = super::bounds::exact_serialized_size(&record).map_err(encoding_failure)?;
            let bytes = self
                .bytes
                .saturating_add(bytes)
                .saturating_add(usize::from(!self.records.is_empty()));
            if self.records.len() == PROVENANCE_RECORD_CAP || bytes > PROVENANCE_BYTE_CAP {
                return Err(WorkError::projection_failure(
                    WorkFailureCause::ProvenanceCapacity,
                ));
            }
            self.bytes = bytes;
            self.seen.insert(record.clone());
            self.records.push(record);
        }
        Ok(())
    }

    fn roots<'a>(
        &mut self,
        app: &AppState,
        batch: Option<&mut crate::privacy::records::BatchDeclaredRecords>,
        roots: impl IntoIterator<Item = &'a WorkRootRef>,
    ) -> Result<(), WorkError> {
        let paths = roots
            .into_iter()
            .flat_map(|root| root.path.iter().chain(root.source_path.iter()))
            .map(PathBuf::from);
        let records = match batch {
            Some(batch) => batch.classify_declared_paths(&app.gcx, paths),
            None => crate::privacy::records::declared_file_records(&app.gcx, paths)
                .map_err(|_| WorkError::policy_unavailable())?,
        };
        self.extend(records)
    }

    fn row(
        &mut self,
        app: &AppState,
        batch: Option<&mut crate::privacy::records::BatchDeclaredRecords>,
        row: &WorkRow,
    ) -> Result<(), WorkError> {
        self.extend(row.source_records.iter().cloned())?;
        self.roots(app, batch, [&row.root])
    }
}

pub(super) fn budget_page(
    app: &AppState,
    snapshot: &WorkSnapshot,
    offset: usize,
    requested_end: usize,
) -> Result<(usize, Vec<refact_privacy::FileRecord>), WorkError> {
    #[cfg(windows)]
    {
        budget_page_with_batch(app, snapshot, offset, requested_end, None)
    }
    #[cfg(not(windows))]
    {
        let mut batch = crate::privacy::records::BatchDeclaredRecords::new(&app.gcx)
            .map_err(|_| WorkError::policy_unavailable())?;
        let result = budget_page_with_batch(app, snapshot, offset, requested_end, Some(&mut batch));
        let finished = batch
            .finish(&app.gcx)
            .map_err(|_| WorkError::policy_unavailable());
        finished?;
        result
    }
}

fn budget_page_with_batch(
    app: &AppState,
    snapshot: &WorkSnapshot,
    offset: usize,
    requested_end: usize,
    mut batch: Option<&mut crate::privacy::records::BatchDeclaredRecords>,
) -> Result<(usize, Vec<refact_privacy::FileRecord>), WorkError> {
    let mut budget = ProvenanceBudget::new();
    let paths = snapshot.selected_paths.iter().cloned().chain(
        snapshot
            .file_provenance
            .iter()
            .map(|entry| PathBuf::from(&entry.path)),
    );
    let records = match batch.as_deref_mut() {
        Some(batch) => batch.classify_declared_paths(&app.gcx, paths),
        None => crate::privacy::records::declared_file_records(&app.gcx, paths)
            .map_err(|_| WorkError::policy_unavailable())?,
    };
    budget.extend(records)?;
    for row in snapshot.detail.iter().chain(snapshot.guard_rows.iter()) {
        budget.row(app, batch.as_deref_mut(), row)?;
    }
    let roots_by_id: HashMap<_, _> = snapshot.roots.iter().map(|root| (&root.id, root)).collect();
    let mut end = offset;
    for index in offset..requested_end {
        let count = budget.records.len();
        let bytes = budget.bytes;
        let result = if snapshot.rows.is_empty() {
            budget.roots(app, batch.as_deref_mut(), [&snapshot.roots[index]])
        } else {
            let row = &snapshot.rows[index];
            budget.row(app, batch.as_deref_mut(), row).and_then(|_| {
                budget.roots(
                    app,
                    batch.as_deref_mut(),
                    roots_by_id.get(&row.root.id).copied(),
                )
            })
        };
        if let Err(error) = result {
            if error.cause != Some(WorkFailureCause::ProvenanceCapacity) || index == offset {
                return Err(error);
            }
            for record in budget.records.drain(count..) {
                budget.seen.remove(&record);
            }
            budget.bytes = bytes;
            break;
        }
        end = index + 1;
    }
    Ok((end, budget.records))
}

pub async fn visible_projection(
    app: &AppState,
    caller: &WorkCaller,
) -> Result<(Arc<WorkCatalog>, String), WorkError> {
    projection(app, caller, false).await
}

pub(super) async fn context_projection(
    app: &AppState,
    caller: &WorkCaller,
) -> Result<(Arc<WorkCatalog>, String), WorkError> {
    projection(app, caller, true).await
}

async fn projection(
    app: &AppState,
    caller: &WorkCaller,
    shared: bool,
) -> Result<(Arc<WorkCatalog>, String), WorkError> {
    #[cfg(test)]
    let _ = PROJECTION_BUILDS.try_with(|count| count.set(count.get() + 1));
    let before = match &caller.model {
        Some(model) => privacy_guard::policy_key(&app.gcx, model)?,
        None => "operator".into(),
    };
    let source = caller
        .source_root
        .to_str()
        .ok_or_else(WorkError::scope_denied)?;
    let base = if shared {
        Some(super::catalog_cache::neutral(app, caller).await?)
    } else {
        None
    };
    let privacy = if shared {
        Some(
            crate::privacy::classification_cache::privacy_key(&app.gcx)
                .map_err(|_| WorkError::policy_unavailable())?,
        )
    } else {
        None
    };
    if let Some(base) = &base {
        if let Some(visible) = super::catalog_cache::visible(app, base, &before) {
            check_disclosure(app, caller, &before)?;
            return Ok((visible, before));
        }
    }
    let mut catalog = if let Some(base) = &base {
        (**base).clone()
    } else {
        build_catalog_for_destination(app, Some(source), caller.model.is_some())
            .await
            .map_err(|_| {
                if caller.model.is_some() {
                    WorkError::unavailable()
                } else {
                    WorkError::projection_failure(WorkFailureCause::ScopedSourceUnavailable)
                }
            })?
    };
    let app_owned = app.clone();
    let caller_owned = caller.clone();
    let expected_disclosure = before.clone();
    let catalog = projection_blocking(move || {
        if let Some(model) = &caller_owned.model {
            if catalog
                .degraded
                .iter()
                .any(|reason| reason == "historical index unavailable")
            {
                return Err(WorkError::unavailable());
            }
            privacy_guard::with_clearance(&app_owned.gcx, model, |clearance| {
                catalog.retain_visible(clearance.clear_rows(catalog.all_rows())?);
                let roots = clearance.clear_roots(catalog.roots.all())?;
                catalog.roots.by_id = roots.into_iter().map(|r| (r.id.clone(), r)).collect();
                catalog.degraded.clear();
                Ok(())
            })?;
            check_disclosure(&app_owned, &caller_owned, &expected_disclosure)?;
        }
        Ok(Arc::new(catalog))
    })
    .await?;
    check_disclosure(app, caller, &before)?;
    if let (Some(base), Some(privacy)) = (base, privacy) {
        if crate::privacy::classification_cache::privacy_key(&app.gcx)
            .map_err(|_| WorkError::policy_unavailable())?
            != privacy
        {
            return Err(WorkError::policy_unavailable());
        }
        super::catalog_cache::save_visible(app, base, before.clone(), privacy, catalog.clone());
    }
    Ok((catalog, before))
}

#[cfg(test)]
tokio::task_local! {
    pub(super) static PROJECTION_BUILDS: std::cell::Cell<usize>;
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
        serde_json::to_writer(&mut output, &(binding, self)).map_err(encoding_failure)?;
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
            .map_err(encoding_failure)?;
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
        let current_records: HashSet<_> = current.source_records.iter().collect();
        if row_scope(row)? != row_scope(current)?
            || current.metadata.source_revision < row.metadata.source_revision
            || !row
                .source_records
                .iter()
                .all(|record| current_records.contains(record))
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
        privacy_guard::with_clearance(&app.gcx, model, |clearance| {
            if clearance.clear_rows(audit)?.len() != expected
                || clearance.clear_roots(snapshot.roots.clone())?.len() != snapshot.roots.len()
            {
                return Err(WorkError::cursor_expired());
            }
            Ok(())
        })?;
        if super::file_provenance::clear_for_model(
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

fn prepare_snapshot(
    caller: &WorkCaller,
    disclosure: &str,
    filter: String,
    snapshot: &mut WorkSnapshot,
) -> ViewBinding {
    snapshot
        .rows
        .sort_by_key(|row| (std::cmp::Reverse(row.updated_at_ms), row.key.to_string()));
    snapshot.roots.sort_by(|a, b| a.id.cmp(&b.id));
    ViewBinding {
        project: super::roots::project_identity(&caller.source_root),
        principal: caller.principal.clone(),
        disclosure: disclosure.to_owned(),
        filter,
    }
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
    let binding = prepare_snapshot(caller, &disclosure, filter, &mut snapshot);
    let provenance = snapshot.file_provenance.clone();
    let mut records = Vec::new();
    let frozen = app.work_views.frozen_page_select(
        binding,
        snapshot,
        request.rows.unwrap_or(settings.overview_default_rows),
        request.cursor.as_deref(),
        &settings,
        now_ms(),
        |saved| revalidate_snapshot(app, caller, &disclosure, catalog, &provenance, saved),
        |saved, offset, end| {
            if caller.model.is_none() {
                return Ok(end);
            }
            let (end, complete) = budget_page(app, saved, offset, end)?;
            records = complete;
            Ok(end)
        },
    )?;
    check_disclosure(app, caller, &disclosure)?;
    let window = frozen.window;
    let snapshot = frozen.snapshot;
    let page_unit = if snapshot.rows.is_empty() && !snapshot.roots.is_empty() {
        "worktrees"
    } else {
        "work"
    };
    if caller.model.is_some() {
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
    let selected = overview_selection(caller, &mut request)?;
    let (catalog, disclosure) = visible_projection(app, caller).await?;
    overview_offloaded(app, caller, request, selected, catalog, disclosure).await
}

fn overview_selection(
    caller: &WorkCaller,
    request: &mut WorkOverviewRequest,
) -> Result<Option<PathBuf>, WorkError> {
    if request.scope == OverviewScope::Root {
        request.root =
            Some(caller.root_chat_id.clone().ok_or_else(|| {
                WorkError::invalid("root scope requires an existing caller chat")
            })?);
    } else if request.root.is_some() {
        return Err(WorkError::invalid("root is not a project-scope filter"));
    }
    request
        .path
        .as_ref()
        .map(|path| resolve_path(path, caller))
        .transpose()
}

pub(super) async fn inventory_cursor_from_projection(
    app: &AppState,
    caller: &WorkCaller,
    catalog: Arc<WorkCatalog>,
    disclosure: &str,
    privacy: &crate::privacy::classification_cache::PrivacyKey,
) -> Result<String, WorkError> {
    let check = || {
        check_disclosure(app, caller, disclosure)?;
        if crate::privacy::classification_cache::privacy_key(&app.gcx)
            .map_err(|_| WorkError::policy_unavailable())?
            != *privacy
        {
            return Err(WorkError::policy_unavailable());
        }
        Ok(())
    };
    check()?;
    let app_owned = app.clone();
    let caller_owned = caller.clone();
    let disclosure_owned = disclosure.to_owned();
    let privacy_owned = privacy.clone();
    let cursor = projection_blocking(move || {
        let mut request = WorkOverviewRequest::default();
        let selected = overview_selection(&caller_owned, &mut request)?;
        let (mut snapshot, filter) =
            overview_snapshot(&caller_owned, &request, selected, &catalog)?;
        // The exact projection already cleared rows and roots; carried paths need their
        // current classification as well as the carried zone authorization.
        if let Some(model) = &caller_owned.model {
            let paths = snapshot
                .rows
                .iter()
                .flat_map(|row| {
                    row.source_records.iter().map(move |record| WorkRootRef {
                        id: String::new(),
                        kind: super::types::WorkRootKind::Project,
                        path: Some(if Path::new(&record.path).is_absolute() {
                            record.path.clone()
                        } else {
                            Path::new(row.root.path.as_deref().unwrap_or(""))
                                .join(&record.path)
                                .to_string_lossy()
                                .into_owned()
                        }),
                        branch: None,
                        project: row.root.project.clone(),
                        source_path: None,
                        presence: Default::default(),
                    })
                })
                .collect::<Vec<_>>();
            let expected = paths.len();
            if privacy_guard::clear_roots(&app_owned.gcx, paths, model)?.len() != expected {
                return Err(WorkError::cursor_expired());
            }
        }
        check_disclosure(&app_owned, &caller_owned, &disclosure_owned)?;
        if crate::privacy::classification_cache::privacy_key(&app_owned.gcx)
            .map_err(|_| WorkError::policy_unavailable())?
            != privacy_owned
        {
            return Err(WorkError::policy_unavailable());
        }
        let binding = prepare_snapshot(&caller_owned, &disclosure_owned, filter, &mut snapshot);
        app_owned.work_views.fresh_cursor(
            binding,
            snapshot,
            &crate::file_activity::settings(),
            now_ms(),
        )
    })
    .await?;
    check()?;
    Ok(cursor)
}

async fn overview_offloaded(
    app: &AppState,
    caller: &WorkCaller,
    request: WorkOverviewRequest,
    selected: Option<PathBuf>,
    catalog: Arc<WorkCatalog>,
    disclosure: String,
) -> Result<WorkOverview, WorkError> {
    let app_owned = app.clone();
    let caller_owned = caller.clone();
    let expected_disclosure = disclosure.clone();
    let result = projection_blocking(move || {
        overview_selected(
            &app_owned,
            &caller_owned,
            request,
            selected,
            &catalog,
            &disclosure,
        )
    })
    .await?;
    check_disclosure(app, caller, &expected_disclosure)?;
    Ok(result)
}

fn overview_selected(
    app: &AppState,
    caller: &WorkCaller,
    request: WorkOverviewRequest,
    selected: Option<PathBuf>,
    catalog: &WorkCatalog,
    disclosure: &str,
) -> Result<WorkOverview, WorkError> {
    let (snapshot, filter) = overview_snapshot(caller, &request, selected, catalog)?;
    page(
        app,
        caller,
        disclosure.to_owned(),
        &request,
        snapshot,
        filter,
        catalog,
    )
    .map(|(page, _)| page)
}

fn overview_snapshot(
    caller: &WorkCaller,
    request: &WorkOverviewRequest,
    selected: Option<PathBuf>,
    catalog: &WorkCatalog,
) -> Result<(WorkSnapshot, String), WorkError> {
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
    Ok((
        WorkSnapshot {
            rows,
            roots,
            selected_paths: selected.into_iter().collect(),
            degraded: catalog.degraded.clone(),
            ..Default::default()
        },
        digest(&(&filter, &request.root, &caller.checkout_root))?,
    ))
}

pub async fn inspect(
    app: &AppState,
    caller: &WorkCaller,
    request: InspectRequest,
) -> Result<Inspection, WorkError> {
    let paths = match &request.selector {
        InspectSelector::Files {
            paths,
        } => {
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
        InspectSelector::Work {
            work_id,
        } => {
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
        InspectSelector::Files {
            ..
        } => {
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
        InspectSelector::Worktree {
            worktree_id,
        } => {
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
    let snapshot = WorkSnapshot {
        rows,
        roots,
        detail: row,
        degraded: catalog.degraded.clone(),
        advisory: advisory.into_iter().collect(),
        wait_graph,
        file_provenance,
        selected_paths: paths,
        guard_rows,
    };
    let app_owned = app.clone();
    let caller_owned = caller.clone();
    let expected_disclosure = disclosure.clone();
    let (page, snapshot) = projection_blocking(move || {
        page(
            &app_owned,
            &caller_owned,
            disclosure,
            &overview_request,
            snapshot,
            filter,
            &catalog,
        )
    })
    .await?;
    check_disclosure(app, caller, &expected_disclosure)?;
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
    let mut budget = ProvenanceBudget::new();
    budget.extend(page.source_records.iter().cloned())?;
    for row in page.rows.iter().chain(detail) {
        budget.row(app, None, row)?;
    }
    budget.roots(app, None, &page.roots)?;
    crate::privacy::records::merge_records(message, budget.records);
    Ok(())
}

#[cfg(all(test, not(windows)))]
mod batching_tests {
    use super::*;
    use crate::privacy::records::BatchDeclaredRecords;

    fn root(id: usize, path: PathBuf) -> WorkRootRef {
        WorkRootRef {
            id: id.to_string(),
            kind: super::super::types::WorkRootKind::Project,
            path: Some(path.to_string_lossy().into_owned()),
            branch: None,
            project: "batching".into(),
            source_path: None,
            presence: Default::default(),
        }
    }

    async fn fixture() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        (AppState::from_gcx(gcx).await, dir)
    }

    #[tokio::test]
    async fn budget_batch_matches_independent_order_and_capacity_rollback() {
        let (app, dir) = fixture().await;
        let mut snapshot = WorkSnapshot {
            selected_paths: vec![dir.path().join("selected"), dir.path().join("selected")],
            roots: (0..260)
                .map(|id| root(id, dir.path().join(format!("root-{id}"))))
                .collect(),
            ..Default::default()
        };
        let independent = budget_page_with_batch(&app, &snapshot, 0, 260, None).unwrap();
        let batched = budget_page(&app, &snapshot, 0, 260).unwrap();
        assert_eq!(batched, independent);
        assert_eq!(batched.0, 255);
        assert_eq!(batched.1.len(), PROVENANCE_RECORD_CAP);
        assert!(batched.1[0].path.ends_with("selected"));
        assert!(batched.1[255].path.ends_with("root-254"));
        assert!(!batched
            .1
            .iter()
            .any(|record| record.path.ends_with("root-255")));
        let next = budget_page(&app, &snapshot, 255, 260).unwrap();
        assert_eq!(
            next,
            budget_page_with_batch(&app, &snapshot, 255, 260, None).unwrap()
        );
        assert!(next.1[1].path.ends_with("root-255"));
        snapshot.selected_paths = (0..256)
            .map(|id| dir.path().join(format!("selected-{id}")))
            .collect();
        assert_eq!(
            budget_page(&app, &snapshot, 0, 1).unwrap_err().cause,
            Some(WorkFailureCause::ProvenanceCapacity)
        );
        assert_eq!(
            budget_page_with_batch(&app, &snapshot, 0, 1, None)
                .unwrap_err()
                .cause,
            Some(WorkFailureCause::ProvenanceCapacity)
        );
        snapshot.selected_paths.clear();
        snapshot.roots.clear();
        assert_eq!(budget_page(&app, &snapshot, 0, 0).unwrap(), (0, Vec::new()));
        app.gcx.privacy_policy_load.write().unwrap().policy =
            Arc::new(refact_privacy::PrivacyPolicy {
                blocked: vec!["[".into()],
                ..Default::default()
            });
        assert_eq!(
            budget_page(&app, &snapshot, 0, 0).unwrap_err().code,
            "policy_unavailable"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn budget_batch_reuses_symlink_classification_until_explicit_invalidation() {
        let (app, dir) = fixture().await;
        let allowed = dir.path().join("allowed");
        let guarded = dir.path().join("guarded");
        let link = dir.path().join("link");
        std::fs::write(&allowed, "allowed").unwrap();
        std::fs::write(&guarded, "guarded").unwrap();
        std::os::unix::fs::symlink(&allowed, &link).unwrap();
        app.gcx.privacy_policy_load.write().unwrap().policy =
            Arc::new(refact_privacy::PrivacyPolicy {
                blocked: vec![guarded.to_string_lossy().into_owned()],
                ..Default::default()
            });
        let root = root(0, link.clone());
        let snapshot = WorkSnapshot {
            roots: vec![root.clone()],
            ..Default::default()
        };
        let before = budget_page(&app, &snapshot, 0, 1).unwrap();
        assert!(before.1.iter().all(|record| record.zone != "blocked"));
        let mut batch = BatchDeclaredRecords::new(&app.gcx).unwrap();
        let mut budget = ProvenanceBudget::new();
        budget.roots(&app, Some(&mut batch), [&root]).unwrap();
        assert_ne!(budget.records.last().unwrap().zone, "blocked");
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&guarded, &link).unwrap();
        budget.roots(&app, Some(&mut batch), [&root]).unwrap();
        assert_eq!(budget.records.len(), 1);
        assert_ne!(budget.records.last().unwrap().zone, "blocked");
        batch.finish(&app.gcx).unwrap();
        assert_eq!(budget_page(&app, &snapshot, 0, 1).unwrap(), before);
        crate::privacy::classification_cache::invalidate(&app.gcx);
        assert!(batch.finish(&app.gcx).is_err());
        let after = budget_page(&app, &snapshot, 0, 1).unwrap();
        assert_eq!(after.0, before.0);
        assert_eq!(after.1.len(), before.1.len());
        assert!(after.1.iter().all(|record| record.zone == "blocked"));
    }
}
