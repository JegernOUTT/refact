use crate::app_state::AppState;
use crate::coordination::{scope::WorkCaller, types::WorkRow};
use super::{
    store::{self, Store},
    types::*,
};

pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    static SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(4)));
    let slot = SLOTS
        .clone()
        .acquire_owned()
        .await
        .map_err(Error::storage)?;
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        f()
    })
    .await
    .map_err(Error::storage)?
}

pub async fn project_caller(app: &AppState, caller: &WorkCaller) -> Result<WorkCaller, Error> {
    let path = caller.source_root.clone();
    let candidate = blocking(move || {
        let root = dunce::canonicalize(&path).map_err(|_| Error::denied())?;
        let repo = match git2::Repository::open(&root) {
            Ok(repo) => repo,
            Err(_) => return Ok((root, None)),
        };
        if !repo.is_worktree() {
            return Ok((root, None));
        }
        let common = dunce::canonicalize(repo.commondir()).map_err(|_| Error::denied())?;
        if common.file_name().is_none_or(|name| name != ".git") {
            return Err(Error::denied());
        }
        let source = common.parent().ok_or_else(Error::denied)?.to_path_buf();
        Ok((root, Some(source)))
    })
    .await?;
    let source = if let Some(source) = candidate.1 {
        let service = crate::worktrees::service::WorktreeService::new_async(
            app.gcx.cache_dir.clone(),
            source,
        )
        .await
        .map_err(|_| Error::denied())?;
        let registry = service.load_registry().await.map_err(|_| Error::denied())?;
        let mut found = None;
        for record in registry.records {
            if tokio::fs::canonicalize(&record.meta.root)
                .await
                .ok()
                .as_deref()
                == Some(candidate.0.as_path())
            {
                if found.is_some() {
                    return Err(Error::denied());
                }
                service
                    .validate_worktree_meta_strict(&record.meta)
                    .await
                    .map_err(|_| Error::denied())?;
                found = Some(service.source_workspace_root().to_path_buf());
            }
        }
        found.ok_or_else(Error::denied)?
    } else {
        candidate.0
    };
    Ok(WorkCaller {
        source_root: source,
        checkout_root: caller.checkout_root.clone(),
        root_chat_id: caller.root_chat_id.clone(),
        principal: caller.principal.clone(),
        model: caller.model.clone(),
    })
}

pub fn canonical_item(row: &WorkRow) -> Option<Item> {
    if let Some(id) = &row.metadata.task_id {
        return Some(Item {
            kind: ItemKind::Task,
            id: id.clone(),
        });
    }
    Some(Item {
        kind: ItemKind::Chat,
        id: row.root_chat_id.as_ref().or(row.chat_id.as_ref())?.clone(),
    })
}
struct Audit(Vec<refact_privacy::FileRecord>);
impl refact_privacy::PrivacyAudited for Audit {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        Ok(self.0.iter().cloned().enumerate().collect())
    }
}
fn clear(
    app: &AppState,
    caller: &WorkCaller,
    records: &[refact_privacy::FileRecord],
) -> Result<(), Error> {
    if let Some(model) = &caller.model {
        let mut records = records.to_vec();
        let paths = records
            .iter()
            .map(|r| {
                let path = std::path::PathBuf::from(&r.path);
                if path.is_absolute() {
                    path
                } else {
                    caller.source_root.join(path)
                }
            })
            .collect::<Vec<_>>();
        records.extend(
            crate::privacy::records::declared_file_records(&app.gcx, paths)
                .map_err(|_| Error::denied())?,
        );
        crate::privacy::destinations::clear_for_model(&app.gcx, Audit(records.to_vec()), model)
            .map_err(|_| Error::denied())?;
    }
    Ok(())
}
pub fn provenance(
    app: &AppState,
    caller: &WorkCaller,
) -> Result<Vec<refact_privacy::FileRecord>, Error> {
    crate::privacy::records::declared_file_records(&app.gcx, [Store::path(&caller.source_root)])
        .map_err(|_| Error::denied())
}
pub async fn clear_store(
    app: &AppState,
    caller: &WorkCaller,
) -> Result<Vec<refact_privacy::FileRecord>, Error> {
    let normalized = project_caller(app, caller).await?;
    let caller = &normalized;
    let mut records = provenance(app, caller)?;
    let root = caller.source_root.clone();
    records.extend(blocking(move || store::provenance(&Store::open(&root)?.conn)).await?);
    let paths = records
        .iter()
        .map(|r| {
            let path = std::path::PathBuf::from(&r.path);
            if path.is_absolute() {
                path
            } else {
                caller.source_root.join(path)
            }
        })
        .collect::<Vec<_>>();
    records.extend(
        crate::privacy::records::declared_file_records(&app.gcx, paths)
            .map_err(|_| Error::denied())?,
    );
    clear(app, caller, &records)?;
    Ok(records)
}
async fn authorize(
    app: &AppState,
    caller: &WorkCaller,
    items: &[Item],
) -> Result<Vec<refact_privacy::FileRecord>, Error> {
    let mut pending = Vec::new();
    let mut records = Vec::new();
    for item in items {
        let live = if item.kind == ItemKind::Chat {
            app.chat.sessions.read().await.get(&item.id).cloned()
        } else {
            None
        };
        if let Some(live) = live {
            let session = live.try_lock().map_err(|_| Error::denied())?;
            let source = crate::coordination::scope::session_source_root(&session)
                .ok_or_else(Error::denied)?;
            if crate::coordination::roots::project_identity(&source)
                != crate::coordination::roots::project_identity(&caller.source_root)
                || session.thread.parent_id.is_some()
                || session
                    .thread
                    .root_chat_id
                    .as_ref()
                    .is_some_and(|id| id != &item.id)
                || session.thread.task_meta.is_some()
            {
                return Err(Error::denied());
            }
            let (provenance, complete) = session
                .work_provenance
                .snapshot(session.messages.len(), &crate::file_activity::settings());
            if caller.model.is_some() && !complete {
                return Err(Error::denied());
            }
            records.extend(provenance);
        } else {
            pending.push(item.clone());
        }
    }
    if !pending.is_empty() {
        records.extend(super::sources::authorize(app, caller, &pending).await?);
    }
    clear(app, caller, &records)?;
    Ok(records)
}
pub async fn taxonomy(
    app: &AppState,
    caller: &WorkCaller,
    kind: TaxonomyKind,
    limit: usize,
    cursor: Option<&str>,
) -> Result<TaxonomyResponse, Error> {
    let normalized = project_caller(app, caller).await?;
    let caller = &normalized;
    clear_store(app, caller).await?;
    let root = caller.source_root.clone();
    let cursor = cursor.map(str::to_owned);
    blocking(move || Store::open(&root)?.taxonomy(kind, limit, cursor.as_deref())).await
}
pub async fn items(
    app: &AppState,
    caller: &WorkCaller,
    request: ItemsRequest,
) -> Result<ItemsResponse, Error> {
    if request.items.len() > 200 {
        return Err(Error::invalid("at most 200 items"));
    }
    let normalized = project_caller(app, caller).await?;
    let caller = &normalized;
    clear_store(app, caller).await?;
    let source_records = authorize(app, caller, &request.items).await?;
    let root = caller.source_root.clone();
    blocking(move || {
        let store = Store::open(&root)?;
        let _tx = store.conn.unchecked_transaction()?;
        let assignments = request
            .items
            .iter()
            .map(|item| store::assignment(&store.conn, item))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ItemsResponse {
            source_records,
            revision: store.revision()?,
            assignments,
        })
    })
    .await
}
pub async fn apply(
    app: &AppState,
    caller: &WorkCaller,
    request: ApplyRequest,
) -> Result<ApplyResponse, Error> {
    let normalized = project_caller(app, caller).await?;
    let caller = &normalized;
    clear_store(app, caller).await?;
    if caller.model.is_some()
        && request
            .operations
            .iter()
            .any(|op| !matches!(op, Op::AssignItem { .. }))
    {
        return Err(Error::invalid(
            "model taxonomy edits require source-bound candidate review",
        ));
    }
    let items = request
        .operations
        .iter()
        .filter_map(|op| match op {
            Op::AssignItem {
                item, ..
            } => Some(item.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let records = authorize(app, caller, &items).await?;
    let root = caller.source_root.clone();
    let gcx = app.gcx.clone();
    let authority = if caller.model.is_some() {
        Authority::Model
    } else {
        Authority::User
    };
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    let guard = super::candidates::CancelReviewOnDrop(cancel.clone());
    let result = blocking(move || {
        super::candidates::ensure_active(&cancel, &gcx.shutdown_flag)?;
        Store::open(&root)?.apply_guarded(&request, authority, &records, || {
            super::candidates::admit_commit(&cancel, &gcx.shutdown_flag)
        })
    })
    .await;
    drop(guard);
    result
}
