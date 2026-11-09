mod actions;
pub(crate) mod automation_channels;
mod commands;
pub mod context;
mod details;
mod document;
mod icon;
mod privacy;
pub mod process_sources;
pub mod protocol;
#[cfg(test)]
mod tests;
mod views;
pub use commands::{execute, Caller, Command};
pub use views::{EventEnvelope, Subscription};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use refact_apps_core::AppStore;

#[derive(Default)]
pub struct AppRuntimes {
    projects: Mutex<HashMap<PathBuf, Arc<AppRuntime>>>,
}
pub struct AppRuntime {
    pub store: AppStore,
    pub root: PathBuf,
    pub views: Arc<Mutex<views::Views>>,
    pub events: Arc<views::EventBus>,
    pub activation: Mutex<()>,
    pub actions: Mutex<actions::ActionRuntime>,
    pub automation: Arc<refact_scheduler_core::sources::AutomationService>,
    pub generic: Arc<crate::scheduler::generic_runtime::GenericAutomationRuntime>,
    forwarder: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub process_sources: Arc<process_sources::ProcessSourceAdapter>,
    process_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl AppRuntimes {
    pub async fn get(
        &self,
        root: PathBuf,
        gcx: crate::global_context::SharedGlobalContext,
    ) -> Result<Arc<AppRuntime>, String> {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(|e| e.to_string())?;
        let mut projects = self.projects.lock().await;
        if gcx.shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
            return Err("app runtime is shutting down".into());
        }
        if let Some(runtime) = projects.get(&root) {
            return Ok(runtime.clone());
        }
        if projects.len() >= 32 {
            return Err("app runtime project capacity reached".into());
        }
        let store = AppStore::open(&root).await?;
        let automation = refact_scheduler_core::sources::AutomationService::new_suspended(&root)?;
        let credentials =
            crate::scheduler::credentials::AutomationCredentialStore::open(&root, None)?;
        automation
            .set_credential_resolver(Some(Arc::new(credentials)))
            .await?;
        // Published manifests, not a first UI fetch, are the source of desired App collectors.
        let listing = store.list().await?;
        let mut published_ids = std::collections::HashSet::new();
        for record in listing.apps {
            if record.active_revision.is_none() {
                continue;
            }
            let bundle = store.published(&record.slug).await?;
            let id = bundle.manifest.app_id.to_string();
            published_ids.insert(id.clone());
            automation
                .configure(&id, bundle.revision, bundle.manifest.sources)
                .await?;
        }
        let journal = refact_scheduler_core::automation::AutomationJournal::open(&root)?;
        for source in journal.list_sources()? {
            if source.app_id != crate::scheduler::generic_runtime::PROJECT_SOURCE_OWNER
                && !published_ids.contains(&source.app_id)
            {
                automation.unconfigure(&source.app_id).await?;
            }
        }
        let generic = crate::scheduler::generic_runtime::GenericAutomationRuntime::new(
            root.clone(),
            &gcx,
            automation.clone(),
        )?;
        generic.tick().await?;
        let process_sources =
            process_sources::ProcessSourceAdapter::new(gcx, root.clone(), automation.clone());
        let process_task = process_sources.start();
        let mut subscription = automation.subscribe();
        let events = Arc::new(views::EventBus::new());
        let forward_events = events.clone();
        let views = Arc::new(Mutex::new(views::Views::default()));
        let demand_views = views.clone();
        let demand_automation = automation.clone();
        let demand_store = store.clone();
        let forwarder = tokio::spawn(async move {
            let mut demands = std::collections::HashSet::new();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                tokio::select! {
                    event=subscription.recv()=>match event {
                        Ok(event)=>forward_events.emit("source",serde_json::to_value(event).unwrap_or_default()),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>forward_events.emit("reset",serde_json::json!({"reason":"source_gap"})),
                        Err(_)=>break,
                    },
                    _=interval.tick()=>{
                        let active=demand_views.lock().await.active();
                        let mut next=std::collections::HashSet::new();
                        for (id,view) in active {
                            let Ok(record)=demand_store.get_by_id(view.app_id).await else{continue};
                            let Ok(bundle)=demand_store.published(&record.slug).await else{continue};
                            if bundle.revision!=view.revision{continue;}
                            for source in bundle.manifest.sources {
                                next.insert((view.app_id.to_string(),source.source_id,format!("view:{id}")));
                            }
                        }
                        for (app,source,consumer) in demands.difference(&next){let _=demand_automation.set_demand(app,source,consumer,false).await;}
                        for (app,source,consumer) in next.difference(&demands){let _=demand_automation.set_demand(app,source,consumer,true).await;}
                        demands=next;
                    }
                }
            }
        });
        automation.activate().await?;
        let runtime = Arc::new(AppRuntime {
            store,
            root: root.clone(),
            views,
            events,
            activation: Mutex::new(()),
            actions: Mutex::new(actions::ActionRuntime::default()),
            automation,
            generic,
            forwarder: Mutex::new(Some(forwarder)),
            process_sources,
            process_task: Mutex::new(Some(process_task)),
        });
        projects.insert(root, runtime.clone());
        Ok(runtime)
    }
    pub async fn shutdown(&self) {
        let projects = std::mem::take(&mut *self.projects.lock().await);
        for runtime in projects.values() {
            runtime.views.lock().await.clear();
            runtime.events.close();
            runtime.process_sources.shutdown().await;
            if let Some(handle) = runtime.process_task.lock().await.take() {
                let _ = handle.await;
            }
            runtime.automation.shutdown().await;
            runtime.generic.shutdown().await;
            if let Some(handle) = runtime.forwarder.lock().await.take() {
                handle.abort();
                let _ = handle.await;
            }
        }
    }
}

pub async fn runtime(
    app: &crate::app_state::AppState,
    caller: &Caller,
) -> Result<Arc<AppRuntime>, String> {
    let root = match caller {
        Caller::Host => {
            crate::coordination::scope::operator_caller(app)
                .await
                .map_err(|e| e.message)?
                .checkout_root
        }
        Caller::HostChat {
            chat_id,
        } => {
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get(chat_id)
                .cloned()
                .ok_or("host chat unavailable")?;
            let model = session.lock().await.thread.model.clone();
            crate::coordination::scope::model_caller(app, chat_id, &model)
                .await
                .map_err(|e| e.message)?
                .checkout_root
        }
        Caller::Tool {
            chat_id,
            model,
        } => {
            let model = match model {
                Some(model) => model.clone(),
                None => {
                    let session = app
                        .chat
                        .sessions
                        .read()
                        .await
                        .get(chat_id)
                        .cloned()
                        .ok_or("caller chat unavailable")?;
                    let model = session.lock().await.thread.model.clone();
                    model
                }
            };
            crate::coordination::scope::model_caller(app, chat_id, &model)
                .await
                .map_err(|e| e.message)?
                .checkout_root
        }
    };
    app.gcx.apps.get(root, app.gcx.clone()).await
}

pub async fn validate_binding(
    app: &crate::app_state::AppState,
    runtime: &AppRuntime,
    app_id: uuid::Uuid,
    chat_id: Option<&str>,
    binding_id: Option<uuid::Uuid>,
) -> Result<(), String> {
    match (chat_id, binding_id) {
        (None, None) => Ok(()),
        (Some(chat_id), Some(binding_id)) => {
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get(chat_id)
                .cloned()
                .ok_or("binding chat unavailable")?;
            let session = session.lock().await;
            let workspace = session
                .thread
                .app_workspace
                .as_ref()
                .and_then(|w| w.supported())
                .cloned()
                .ok_or("unsupported or missing app workspace")?;
            workspace.validate()?;
            let root = session
                .thread
                .worktree
                .as_ref()
                .map(|w| w.root.clone())
                .or_else(|| session.work_project_root.clone())
                .ok_or("binding has no project")?;
            drop(session);
            let root = tokio::fs::canonicalize(root)
                .await
                .map_err(|_| "binding root unavailable")?;
            if root != runtime.root {
                return Err("binding belongs to another checkout".into());
            }
            if !workspace
                .bindings
                .iter()
                .any(|b| b.binding_id == binding_id && b.app_id == app_id)
            {
                return Err("binding does not target app".into());
            }
            Ok(())
        }
        _ => Err("chat_id and binding_id must be supplied together".into()),
    }
}

pub async fn document(
    app: &crate::app_state::AppState,
    app_id: uuid::Uuid,
    chat_id: Option<String>,
    binding_id: Option<uuid::Uuid>,
) -> Result<serde_json::Value, String> {
    let host = chat_id
        .as_ref()
        .map(|chat_id| Caller::HostChat {
            chat_id: chat_id.clone(),
        })
        .unwrap_or(Caller::Host);
    let runtime = runtime(app, &host).await?;
    validate_binding(
        app,
        &runtime,
        app_id,
        if binding_id.is_some() {
            chat_id.as_deref()
        } else {
            None
        },
        binding_id,
    )
    .await?;
    let _activation = runtime.activation.lock().await;
    let record = runtime.store.get_by_id(app_id).await?;
    let bundle = runtime.store.published(&record.slug).await?;
    runtime
        .automation
        .configure(
            &app_id.to_string(),
            bundle.revision.clone(),
            bundle.manifest.sources.clone(),
        )
        .await?;
    let html = document::inline(&bundle)?;
    let files: std::collections::BTreeMap<_, _> = bundle
        .files
        .iter()
        .filter_map(|(k, v)| String::from_utf8(v.clone()).ok().map(|v| (k.clone(), v)))
        .collect();
    let state = runtime
        .store
        .read_state(
            &record.slug,
            binding_id
                .map(|binding_id| refact_apps_core::StateScope::Binding {
                    binding_id,
                })
                .unwrap_or(refact_apps_core::StateScope::Standalone),
        )
        .await?;
    Ok(
        serde_json::json!({"app_id":app_id,"revision":bundle.revision,"html":html,"files":files,"state":state,"sources":bundle.manifest.sources,"manifest":bundle.manifest}),
    )
}

pub async fn preflight_tool_records(
    app: &crate::app_state::AppState,
    caller: &Caller,
) -> Result<Vec<refact_privacy::FileRecord>, String> {
    let runtime = runtime(app, caller).await?;
    privacy::preflight_metadata(app, &runtime, caller).await
}

/// Trusted tool adapter entrypoint: call only after the normal confirmation pipeline succeeds.
/// HTTP has no route to this capability.
pub async fn execute_approved_source_control(
    app: &crate::app_state::AppState,
    caller: Caller,
    app_id: uuid::Uuid,
    source_id: String,
    operation: String,
    expected_revision: &str,
) -> Result<serde_json::Value, String> {
    if !matches!(caller, Caller::Tool { .. }) {
        return Err("source grants require confirmed tool invocation".into());
    }
    let runtime = runtime(app, &caller).await?;
    let _activation = runtime.activation.lock().await;
    privacy::preflight_metadata(app, &runtime, &caller).await?;
    let record = runtime.store.get_by_id(app_id).await?;
    let bundle = runtime.store.published(&record.slug).await?;
    if bundle.revision != expected_revision {
        return Err("active revision conflict".into());
    }
    let source = bundle
        .manifest
        .sources
        .iter()
        .find(|s| s.source_id == source_id)
        .ok_or("source not declared")?;
    runtime
        .automation
        .configure(
            &app_id.to_string(),
            bundle.revision.clone(),
            bundle.manifest.sources.clone(),
        )
        .await?;
    if matches!(
        source.driver,
        refact_apps_core::SourceDriver::Process { .. }
    ) && operation == "grant"
    {
        let Caller::Tool {
            chat_id, ..
        } = &caller
        else {
            unreachable!()
        };
        runtime
            .process_sources
            .sync(
                &app_id.to_string(),
                &bundle.revision,
                std::slice::from_ref(source),
                chat_id,
            )
            .await?;
    }
    if operation == "revoke" {
        runtime
            .process_sources
            .revoke(&app_id.to_string(), &source_id)
            .await;
    }
    match operation.as_str() {
        "grant" => {
            crate::scheduler::source_authorization::authorize_source_bindings(
                &runtime.root,
                &runtime.automation,
                &app_id.to_string(),
                &source_id,
                &source.driver,
            )
            .await?;
            runtime
                .automation
                .set_grant(&app_id.to_string(), &source_id, true)
                .await?
        }
        "revoke" => {
            runtime
                .automation
                .set_grant(&app_id.to_string(), &source_id, false)
                .await?
        }
        "pause" => {
            runtime
                .automation
                .pause(&app_id.to_string(), &source_id, true)
                .await?
        }
        "resume" => {
            runtime
                .automation
                .pause(&app_id.to_string(), &source_id, false)
                .await?
        }
        "start" => {
            let digest = runtime
                .automation
                .source_config_digest(&app_id.to_string(), &source_id)
                .await?;
            runtime
                .automation
                .set_source_control(
                    &app_id.to_string(),
                    &source_id,
                    &digest,
                    refact_scheduler_core::automation::SourceControl {
                        background: true,
                        resume_after_restart: true,
                        ..Default::default()
                    },
                )
                .await?
        }
        "stop" => {
            let digest = runtime
                .automation
                .source_config_digest(&app_id.to_string(), &source_id)
                .await?;
            runtime
                .automation
                .set_source_control(
                    &app_id.to_string(),
                    &source_id,
                    &digest,
                    refact_scheduler_core::automation::SourceControl::default(),
                )
                .await?
        }
        _ => return Err("unsupported source operation".into()),
    }
    Ok(serde_json::json!({"status":"applied","source_id":source_id,"operation":operation}))
}

/// This capability is available only to the confirmed Rust tool adapter, never HTTP JSON.
pub async fn execute_approved_endpoint_grant(
    app: &crate::app_state::AppState,
    caller: Caller,
    app_id: uuid::Uuid,
    source_id: String,
    expected_revision: &str,
    origin: String,
    allowed_private_ips: Vec<std::net::IpAddr>,
) -> Result<serde_json::Value, String> {
    if !matches!(caller, Caller::Tool { .. }) {
        return Err("endpoint grant requires confirmed tool invocation".into());
    }
    let runtime = runtime(app, &caller).await?;
    let _activation = runtime.activation.lock().await;
    privacy::preflight_metadata(app, &runtime, &caller).await?;
    let record = runtime.store.get_by_id(app_id).await?;
    let bundle = runtime.store.published(&record.slug).await?;
    if bundle.revision != expected_revision {
        return Err("active revision conflict".into());
    }
    let source = bundle
        .manifest
        .sources
        .iter()
        .find(|s| s.source_id == source_id)
        .ok_or("source not declared")?;
    let url = match &source.driver {
        refact_apps_core::SourceDriver::HttpPoll {
            url, ..
        }
        | refact_apps_core::SourceDriver::Sse {
            url,
        }
        | refact_apps_core::SourceDriver::Websocket {
            url,
        } => url,
        _ => return Err("source is not network driven".into()),
    };
    let actual = reqwest::Url::parse(url)
        .map_err(|_| "invalid source endpoint")?
        .origin()
        .ascii_serialization();
    if actual != origin {
        return Err("confirmed origin does not match source endpoint".into());
    }
    runtime
        .automation
        .configure(
            &app_id.to_string(),
            bundle.revision,
            bundle.manifest.sources.clone(),
        )
        .await?;
    runtime
        .automation
        .set_endpoint_grant(
            &app_id.to_string(),
            &source_id,
            Some(refact_scheduler_core::sources::EndpointGrant {
                origin: origin.clone(),
                allowed_private_ips,
            }),
        )
        .await?;
    runtime
        .automation
        .set_grant(&app_id.to_string(), &source_id, true)
        .await?;
    Ok(serde_json::json!({"status":"applied","source_id":source_id,"origin":origin}))
}
