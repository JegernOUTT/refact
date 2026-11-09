use crate::app_state::AppState;
use refact_privacy::{Attribution, FileRecord, PrivacyAudited, PrivacyAuditError};
struct Audit(Vec<FileRecord>);
impl PrivacyAudited for Audit {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        Ok(self.0.iter().cloned().map(|r| (0, r)).collect())
    }
}
/// Conservative catalog-wide clearance before any tool output is materialized.
/// App-provided JSON is never accepted as provenance.
pub async fn preflight(
    app: &AppState,
    runtime: &super::AppRuntime,
    caller: &super::Caller,
) -> Result<Vec<FileRecord>, String> {
    preflight_mode(app, runtime, caller, true).await
}
pub async fn preflight_metadata(
    app: &AppState,
    runtime: &super::AppRuntime,
    caller: &super::Caller,
) -> Result<Vec<FileRecord>, String> {
    preflight_mode(app, runtime, caller, false).await
}
async fn preflight_mode(
    app: &AppState,
    runtime: &super::AppRuntime,
    caller: &super::Caller,
    include_data: bool,
) -> Result<Vec<FileRecord>, String> {
    let super::Caller::Tool {
        chat_id,
        model,
    } = caller
    else {
        return Ok(vec![]);
    };
    let model = match model {
        Some(m) => m.clone(),
        None => {
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get(chat_id)
                .cloned()
                .ok_or("caller unavailable")?;
            let m = session.lock().await.thread.model.clone();
            m
        }
    };
    let scope = crate::coordination::scope::model_caller(app, chat_id, &model)
        .await
        .map_err(|e| e.message)?;
    let policy = app
        .gcx
        .privacy_policy_load
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .policy
        .clone();
    let compiled = policy.compile().map_err(|_| "privacy policy unavailable")?;
    let mut records = vec![];
    let bindings = {
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .ok_or("caller unavailable")?;
        let session = session.lock().await;
        session
            .thread
            .app_workspace
            .as_ref()
            .and_then(|w| w.supported())
            .map(|w| w.bindings.clone())
            .unwrap_or_default()
    };
    for record in runtime.store.list().await?.apps {
        if include_data {
            records.extend(
                runtime
                    .process_sources
                    .known_records(&record.manifest.app_id.to_string())
                    .await?,
            );
        }
        let binding_files = bindings
            .iter()
            .filter(|b| include_data && b.app_id == record.manifest.app_id)
            .map(|b| format!(".local/state/binding-{}.json", b.binding_id));
        for file in record
            .files
            .into_iter()
            .chain(if include_data {
                vec![
                    ".local/state/shared.json".into(),
                    ".local/state/standalone.json".into(),
                ]
            } else {
                vec![]
            })
            .chain(binding_files)
        {
            let path = runtime
                .root
                .join(".refact/apps")
                .join(&record.slug)
                .join(file);
            let zone = compiled
                .zone_for_path_with_roots(&path, [runtime.root.as_path()])
                .name
                .clone();
            records.push(FileRecord {
                path: path.to_string_lossy().into_owned(),
                zone,
                attribution: Attribution::Declared,
            });
        }
    }
    crate::privacy::destinations::clear_for_model(
        &app.gcx,
        Audit(records.clone()),
        scope.model.as_ref().ok_or("caller model unavailable")?,
    )
    .map_err(|r| r.model_facing())?;
    Ok(records)
}
