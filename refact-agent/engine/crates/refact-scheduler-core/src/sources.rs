use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::Duration;

use refact_apps_core::{SourceDefinition, SourceDriver};
use futures_util::{FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use crate::automation::{
    AutomationJournal, ProjectAutomationLease, GrantRecord, SourceControl, IngestRequest,
};
#[path = "source_drivers/policy.rs"]
mod policy;
pub use policy::SourcePolicy;
#[path = "source_drivers/credentials.rs"]
mod credentials;
pub use credentials::{CredentialBinding, CredentialContext, SourceCredentialResolver};
#[path = "source_drivers/transform.rs"]
mod transform;
pub use transform::{apply_transform, TransformSpec};
#[path = "source_drivers/file_tail.rs"]
mod file_tail;
pub use file_tail::{FileGrant, RotationPolicy};
#[path = "source_drivers/sse.rs"]
mod sse;
use serde_json::{json, Value};
use tokio::sync::{broadcast, watch, Mutex, Notify};
use tokio::task::JoinHandle;

const EVENT_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Passive,
    Running,
    Data,
    Unsupported,
    Error,
    Stale,
    Gap,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceEvent {
    pub app_id: String,
    pub source_id: String,
    pub channel: String,
    pub generation: String,
    pub seq: u64,
    pub status: SourceStatus,
    pub data: Option<Value>,
    pub health: Value,
}

pub struct SourceSubscription(broadcast::Receiver<SourceEvent>);

impl SourceSubscription {
    /// Lagged(n) is a mandatory resnapshot signal, not silently dropped data.
    pub async fn recv(&mut self) -> Result<SourceEvent, broadcast::error::RecvError> {
        self.0.recv().await
    }
}

/// An admitted refresh owns only an event subscription, never lifecycle locks.
pub struct RefreshWait {
    events: broadcast::Receiver<SourceEvent>,
    service: Weak<AutomationService>,
    app: String,
    id: String,
    generation: String,
    seq: u64,
}

impl RefreshWait {
    pub async fn wait(mut self) -> Result<SourceEvent, String> {
        tokio::time::timeout(Duration::from_secs(35), async {
            loop {
                let event = self
                    .events
                    .recv()
                    .await
                    .map_err(|_| "refresh event stream lost")?;
                if event.app_id != self.app || event.source_id != self.id {
                    continue;
                }
                if event.generation != self.generation
                    || matches!(event.status, SourceStatus::Passive)
                {
                    return Err("source changed or became passive during refresh".to_owned());
                }
                if event.seq > self.seq {
                    match event.status {
                        SourceStatus::Data => {
                            let service =
                                self.service.upgrade().ok_or("source service unavailable")?;
                            let state = service.state.lock().await;
                            let source = state
                                .sources
                                .get(&(self.app.clone(), self.id.clone()))
                                .ok_or("source no longer configured")?;
                            if state.closed || state_closed_source(source, &self.generation) {
                                return Err(
                                    "source changed or became passive during refresh".into()
                                );
                            }
                            return Ok(event);
                        }
                        SourceStatus::Error => return Err("source refresh read failed".to_owned()),
                        _ => {}
                    }
                }
            }
        })
        .await
        .map_err(|_| "source refresh timed out".to_owned())?
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointGrant {
    pub origin: String,
    pub allowed_private_ips: Vec<std::net::IpAddr>,
}

struct Source {
    definition: SourceDefinition,
    policy: SourcePolicy,
    config_digest: String,
    source_generation: String,
    background: bool,
    resume_after_restart: bool,
    expires_at_ms: Option<u64>,
    last_data_at: Option<u64>,
    started_at: u64,
    retry_at: Option<u64>,
    error_class: Option<String>,
    checkpoint: Option<String>,
    upstream_event_id: Option<String>,
    pending_checkpoint: Option<Option<String>>,
    last_event_type: Option<String>,
    generation: String,
    seq: u64,
    status: SourceStatus,
    last_data: Option<Value>,
    granted: bool,
    endpoint_grant: Option<EndpointGrant>,
    file_grant: Option<FileGrant>,
    paused: bool,
    consumers: HashSet<String>,
    refresh: Arc<Notify>,
    refresh_pending: bool,
    task: Option<(watch::Sender<bool>, JoinHandle<()>)>,
}

#[derive(Default)]
struct State {
    sources: HashMap<(String, String), Source>,
    closed: bool,
    suspended: bool,
    revisions: HashMap<String, String>,
}

pub type SourceAuthorityValidator = dyn Fn(&str, &str) -> Result<(), String> + Send + Sync;

pub struct AutomationService {
    state: Mutex<State>,
    authority_validator: StdMutex<Option<Arc<SourceAuthorityValidator>>>,
    events: broadcast::Sender<SourceEvent>,
    _lease: Arc<ProjectAutomationLease>,
    journal: AutomationJournal,
    credential_resolver: StdMutex<Option<Arc<dyn SourceCredentialResolver>>>,
    file_root: cap_std::fs::Dir,
}

impl AutomationService {
    pub fn new(project_root: impl AsRef<Path>) -> Result<Arc<Self>, String> {
        Self::new_inner(project_root, false)
    }

    /// Host startup must reconcile manifests and resolver capabilities before activation.
    pub fn new_suspended(project_root: impl AsRef<Path>) -> Result<Arc<Self>, String> {
        Self::new_inner(project_root, true)
    }

    fn new_inner(project_root: impl AsRef<Path>, suspended: bool) -> Result<Arc<Self>, String> {
        static SERVICES: OnceLock<StdMutex<HashMap<PathBuf, Weak<AutomationService>>>> =
            OnceLock::new();
        let root = std::fs::canonicalize(project_root).map_err(|e| e.to_string())?;
        let mut services = SERVICES
            .get_or_init(Default::default)
            .lock()
            .map_err(|e| e.to_string())?;
        services.retain(|_, service| service.strong_count() > 0);
        if let Some(service) = services.get(&root).and_then(Weak::upgrade) {
            return Ok(service);
        }
        let lease = ProjectAutomationLease::acquire(&root)?;
        let journal = AutomationJournal::open(&root)?;
        journal.activate_owner(&lease)?;
        let mut restored = State {
            suspended,
            ..Default::default()
        };
        for record in journal.list_sources()? {
            let definition: SourceDefinition =
                serde_json::from_value(record.compiled_spec["definition"].clone())
                    .map_err(|e| e.to_string())?;
            let policy: SourcePolicy =
                serde_json::from_value(record.compiled_spec["policy"].clone())
                    .map_err(|e| e.to_string())?;
            policy.validate()?;
            let key = (record.app_id.clone(), record.source_id.clone());
            let runtime = record.runtime;
            let background = runtime.control.background
                && runtime.control.resume_after_restart
                && !runtime.control.expires_at_ms.is_some_and(|t| t <= now_ms());
            let mut consumers = HashSet::new();
            if background {
                consumers.insert("__background_lease".into());
            }
            // View demand is never authority to resume after process restart.
            journal.set_source_control(
                &key.0,
                &key.1,
                &SourceControl {
                    demand: background,
                    background,
                    resume_after_restart: runtime.control.resume_after_restart,
                    expires_at_ms: runtime.control.expires_at_ms,
                    ..runtime.control.clone()
                },
            )?;
            let epoch = journal
                .source_state(&key.0, &key.1)?
                .ok_or("missing source")?
                .source_generation;
            restored.revisions.insert(key.0.clone(), "recovered".into());
            restored.sources.insert(
                key.clone(),
                Source {
                    definition,
                    policy,
                    config_digest: runtime.config_digest,
                    source_generation: epoch,
                    background,
                    resume_after_restart: runtime.control.resume_after_restart,
                    expires_at_ms: runtime.control.expires_at_ms,
                    last_data_at: None,
                    started_at: now_ms(),
                    retry_at: None,
                    error_class: None,
                    checkpoint: journal.checkpoint(&key.0, &key.1)?,
                    upstream_event_id: None,
                    pending_checkpoint: None,
                    last_event_type: None,
                    generation: uuid::Uuid::new_v4().to_string(),
                    seq: 0,
                    status: SourceStatus::Passive,
                    last_data: journal.latest(&key.0, &key.1)?,
                    granted: runtime.approved,
                    file_grant: runtime
                        .grant
                        .as_ref()
                        .and_then(|g| g.endpoint_grant.as_ref())
                        .and_then(|g| g.get("file_grant"))
                        .and_then(|g| serde_json::from_value(g.clone()).ok()),
                    endpoint_grant: runtime
                        .grant
                        .and_then(|g| g.endpoint_grant)
                        .and_then(|g| serde_json::from_value(g).ok()),
                    paused: runtime.control.paused,
                    consumers,
                    refresh: Arc::new(Notify::new()),
                    refresh_pending: false,
                    task: None,
                },
            );
        }
        let service = Arc::new(Self {
            _lease: lease,
            journal,
            state: Mutex::new(restored),
            authority_validator: StdMutex::new(None),
            events: broadcast::channel(EVENT_CAPACITY).0,
            credential_resolver: StdMutex::new(None),
            file_root: cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority())
                .map_err(|_| "project capability unavailable")?,
        });
        {
            let mut state = service.state.try_lock().map_err(|e| e.to_string())?;
            for (key, source) in state.sources.iter_mut() {
                if !suspended && source.granted && !source.paused && source.background {
                    match source.definition.driver {
                        SourceDriver::Interval {
                            ..
                        }
                        | SourceDriver::HttpPoll {
                            ..
                        }
                        | SourceDriver::Sse {
                            ..
                        }
                        | SourceDriver::FileTail {
                            ..
                        }
                        | SourceDriver::Websocket {
                            ..
                        } => {
                            if tokio::runtime::Handle::try_current().is_err() {
                                return Err("background recovery requires Tokio runtime".into());
                            }
                            service.start_task(key, source);
                        }
                        _ => source.status = SourceStatus::Running,
                    }
                }
            }
        }
        services.insert(root, Arc::downgrade(&service));
        Ok(service)
    }

    /// Call only after trusted host startup reconciliation. Does not confer grants.
    pub async fn activate(self: &Arc<Self>) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if state.closed {
            return Err("automation service shut down".into());
        }
        if !state.suspended {
            return Ok(());
        }
        state.suspended = false;
        for (key, source) in &mut state.sources {
            if source.granted && !source.paused && source_is_demanded(source) {
                match source.definition.driver {
                    SourceDriver::Manual
                    | SourceDriver::Internal {
                        ..
                    }
                    | SourceDriver::Process {
                        ..
                    } => self.emit(key, source, SourceStatus::Running, None),
                    _ => self.start_task(key, source),
                }
            }
        }
        Ok(())
    }

    /// Trusted host-only canonical authority fence. Must not await or reenter this service.
    pub fn set_authority_validator(&self, validator: Arc<SourceAuthorityValidator>) {
        *self
            .authority_validator
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(validator);
    }

    fn check_authority(&self, key: &(String, String), source: &mut Source) -> bool {
        let validator = self
            .authority_validator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if validator.as_ref().is_none_or(|v| v(&key.0, &key.1).is_ok()) {
            return true;
        }
        source.granted = false;
        source.paused = true;
        if let Some((cancel, _)) = &source.task {
            let _ = cancel.send(true);
        }
        source.pending_checkpoint = None;
        source.error_class = Some("canonical_authority_denied".into());
        if self
            .journal
            .grant_source(&GrantRecord {
                app_id: key.0.clone(),
                source_id: key.1.clone(),
                config_digest: source.config_digest.clone(),
                endpoint_grant: None,
                credential_refs: Vec::new(),
                granted: false,
            })
            .is_err()
        {
            source.error_class = Some("durable_revoke_failed".into());
        }
        self.emit(key, source, SourceStatus::Error, None);
        false
    }

    pub fn subscribe(&self) -> SourceSubscription {
        SourceSubscription(self.events.subscribe())
    }

    /// Host-only. First installation permits recovery; replacing an installed
    /// resolver revokes approval before any new credential capability is used.
    pub async fn set_credential_resolver(
        self: &Arc<Self>,
        resolver: Option<Arc<dyn SourceCredentialResolver>>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if state.closed {
            return Err("automation service shut down".into());
        }
        let replacing = {
            let installed = self
                .credential_resolver
                .lock()
                .map_err(|_| "resolver lock")?;
            if let (Some(old), Some(new)) = (installed.as_ref(), resolver.as_ref()) {
                if Arc::ptr_eq(old, new) {
                    return Ok(());
                }
            }
            installed.is_some()
        };
        for (key, source) in &mut state.sources {
            if source.policy.credential_bindings.is_empty() {
                continue;
            }
            stop(source).await;
            if replacing {
                source.granted = false;
                self.journal.grant_source(&GrantRecord {
                    app_id: key.0.clone(),
                    source_id: key.1.clone(),
                    config_digest: source.config_digest.clone(),
                    endpoint_grant: source
                        .endpoint_grant
                        .as_ref()
                        .map(|g| serde_json::to_value(g).unwrap()),
                    credential_refs: source
                        .policy
                        .credential_bindings
                        .iter()
                        .map(|b| b.reference.clone())
                        .collect(),
                    granted: false,
                })?;
            }
        }
        *self
            .credential_resolver
            .lock()
            .map_err(|_| "resolver lock")? = resolver;
        let suspended = state.suspended;
        for (key, source) in &mut state.sources {
            if !suspended
                && !source.policy.credential_bindings.is_empty()
                && source.granted
                && !source.paused
                && source_is_demanded(source)
            {
                self.start_task(key, source);
            }
        }
        Ok(())
    }

    async fn credential_headers(
        &self,
        key: &(String, String),
        generation: &str,
        raw: &str,
    ) -> Result<reqwest::header::HeaderMap, String> {
        let state = self.state.lock().await;
        let source = state.sources.get(key).ok_or("source unavailable")?;
        if state_closed_source(source, generation) {
            return Err("source stopped".into());
        }
        let origin = reqwest::Url::parse(raw)
            .map_err(|_| "invalid endpoint")?
            .origin()
            .ascii_serialization();
        let context = CredentialContext {
            app_id: &key.0,
            source_id: &key.1,
            config_digest: &source.config_digest,
            origin: &origin,
        };
        let resolver = self
            .credential_resolver
            .lock()
            .map_err(|_| "resolver lock")?;
        credentials::headers(
            resolver.as_deref(),
            &context,
            &source.policy.credential_bindings,
        )
    }

    pub async fn configure(
        self: &Arc<Self>,
        app_id: &str,
        revision: String,
        definitions: Vec<SourceDefinition>,
    ) -> Result<(), String> {
        if app_id.is_empty()
            || app_id.len() > 128
            || revision.is_empty()
            || revision.len() > 256
            || definitions.len() > 128
        {
            return Err("invalid app configuration bounds".into());
        }
        let mut ids = HashSet::new();
        for definition in &definitions {
            refact_apps_core::validate_identifier(&definition.source_id)?;
            refact_apps_core::validate_identifier(&definition.channel)?;
            if let SourceDriver::FileTail {
                path,
                every_ms,
            } = &definition.driver
            {
                file_tail::validate_path(path)?;
                if *every_ms < 1000 {
                    return Err("file tail interval must be >=1000ms".into());
                }
            }
            if !ids.insert(definition.source_id.clone()) {
                return Err("duplicate source id".into());
            }
        }
        let mut state = self.state.lock().await;
        if state.closed {
            return Err("automation service shut down".into());
        }
        if !state.revisions.contains_key(app_id) && state.revisions.len() >= 256 {
            return Err("automation app limit".into());
        }
        state.revisions.insert(app_id.to_owned(), revision.clone());
        let old = state
            .sources
            .keys()
            .filter(|(app, _)| app == app_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in old {
            if definitions.iter().any(|d| {
                d.source_id == key.1
                    && serde_json::to_value(d).ok()
                        == serde_json::to_value(&state.sources[&key].definition).ok()
            }) {
                continue;
            }
            if let Some(source) = state.sources.get_mut(&key) {
                stop(source).await;
                source.granted = false;
                source.paused = true;
                if let Err(error) = self.journal.remove_source(&key.0, &key.1) {
                    source.error_class = Some("durable_revoke_failed".into());
                    self.emit(&key, source, SourceStatus::Error, None);
                    return Err(error);
                }
                self.emit(&key, source, SourceStatus::Passive, None);
                state.sources.remove(&key);
            }
        }
        for definition in definitions {
            let key = (app_id.to_owned(), definition.source_id.clone());
            if state.sources.contains_key(&key) {
                continue;
            }
            let policy = SourcePolicy::default();
            let compiled = json!({"definition":definition,"policy":policy});
            let config_digest = crate::automation::digest_json(&compiled)?;
            self.journal
                .reconcile_app_source(app_id, &key.1, &config_digest, &compiled)?;
            let runtime = self
                .journal
                .source_state(app_id, &key.1)?
                .ok_or("missing reconciled source")?;
            let mut source = Source {
                policy,
                config_digest,
                source_generation: runtime.source_generation.clone(),
                background: runtime.control.background
                    && runtime.control.resume_after_restart
                    && !runtime.control.expires_at_ms.is_some_and(|t| t <= now_ms()),
                resume_after_restart: runtime.control.resume_after_restart,
                expires_at_ms: runtime.control.expires_at_ms,
                last_data_at: None,
                started_at: now_ms(),
                retry_at: None,
                error_class: None,
                checkpoint: None,
                upstream_event_id: None,
                pending_checkpoint: None,
                last_event_type: None,
                definition,
                generation: uuid::Uuid::new_v4().to_string(),
                seq: 0,
                status: SourceStatus::Passive,
                last_data: None,
                granted: runtime.approved,
                file_grant: None,
                endpoint_grant: runtime
                    .grant
                    .and_then(|g| g.endpoint_grant)
                    .and_then(|g| serde_json::from_value(g).ok()),
                paused: runtime.control.paused,
                consumers: HashSet::new(),
                refresh: Arc::new(Notify::new()),
                refresh_pending: false,
                task: None,
            };
            self.emit(&key, &mut source, SourceStatus::Passive, None);
            if source.background {
                source.consumers.insert("__background_lease".into());
            }
            state.sources.insert(key, source);
        }
        Ok(())
    }

    pub async fn source_policy(&self, app: &str, id: &str) -> Result<SourcePolicy, String> {
        let state = self.state.lock().await;
        state
            .sources
            .get(&(app.into(), id.into()))
            .map(|s| s.policy.clone())
            .ok_or_else(|| "source not configured".into())
    }

    /// Trusted host-only policy port. Execution policy changes revoke approval before restart.
    pub async fn set_policy(
        self: &Arc<Self>,
        app: &str,
        id: &str,
        policy: SourcePolicy,
    ) -> Result<(), String> {
        policy.validate()?;
        let mut state = self.state.lock().await;
        if state.closed {
            return Err("automation service shut down".into());
        }
        let source = state
            .sources
            .get_mut(&(app.into(), id.into()))
            .ok_or("source not configured")?;
        if (policy.replay_capable || policy.unique_event_ids)
            && !matches!(source.definition.driver, SourceDriver::Sse { .. })
        {
            return Err("replay capability is supported only for SSE upstream IDs".into());
        }
        let compiled = json!({"definition":source.definition,"policy":policy});
        let digest = crate::automation::digest_json(&compiled)?;
        if digest == source.config_digest {
            return Ok(());
        }
        stop(source).await;
        source.granted = false;
        source.endpoint_grant = None;
        source.file_grant = None;
        source.background = false;
        source.consumers.clear();
        source.checkpoint = None;
        source.last_data = None;
        source.generation = uuid::Uuid::new_v4().to_string();
        self.journal
            .reconcile_app_source(app, id, &digest, &compiled)?;
        source.policy = policy;
        source.config_digest = digest;
        source.source_generation = self
            .journal
            .source_state(app, id)?
            .ok_or("missing source")?
            .source_generation;
        self.emit(
            &(app.into(), id.into()),
            source,
            SourceStatus::Passive,
            None,
        );
        Ok(())
    }

    /// This method is a trusted host control port. Manifest permissions never call it.
    pub async fn set_grant(
        self: &Arc<Self>,
        app: &str,
        source: &str,
        granted: bool,
    ) -> Result<(), String> {
        self.control(app, source, |s| s.granted = granted).await
    }

    /// Host-only exact file authorization, persisted bound to the source digest.
    pub async fn set_file_grant(
        self: &Arc<Self>,
        app: &str,
        id: &str,
        grant: Option<FileGrant>,
    ) -> Result<(), String> {
        {
            let mut state = self.state.lock().await;
            if state.closed {
                return Err("automation service shut down".into());
            }
            let source = state
                .sources
                .get_mut(&(app.into(), id.into()))
                .ok_or("source not configured")?;
            let SourceDriver::FileTail {
                path, ..
            } = &source.definition.driver
            else {
                return Err("not a file tail source".into());
            };
            if let Some(grant) = &grant {
                file_tail::validate_path(&grant.path)?;
                if &grant.path != path {
                    return Err("file grant path mismatch".into());
                }
            }
            stop(source).await;
            source.file_grant = grant;
        }
        self.control(app, id, |_| {}).await
    }

    /// Explicit host-approved origin and private IP set; never constructed from manifest permissions.
    pub async fn set_endpoint_grant(
        self: &Arc<Self>,
        app: &str,
        source: &str,
        grant: Option<EndpointGrant>,
    ) -> Result<(), String> {
        let key = (app.to_owned(), source.to_owned());
        {
            let mut state = self.state.lock().await;
            if state.closed {
                return Err("automation service shut down".into());
            }
            let source = state.sources.get_mut(&key).ok_or("source not configured")?;
            if let Some(grant) = &grant {
                let raw = match &source.definition.driver {
                    SourceDriver::HttpPoll {
                        url, ..
                    }
                    | SourceDriver::Sse {
                        url,
                    }
                    | SourceDriver::Websocket {
                        url,
                    } => url,
                    _ => return Err("not a network source".into()),
                };
                let origin = reqwest::Url::parse(raw)
                    .map_err(|e| e.to_string())?
                    .origin()
                    .ascii_serialization();
                if origin != grant.origin || grant.allowed_private_ips.len() > 32 {
                    return Err("endpoint grant mismatch".into());
                }
            }
            stop(source).await;
            source.endpoint_grant = grant;
        }
        self.control(app, source, |_| {}).await
    }

    pub async fn source_config_digest(&self, app: &str, id: &str) -> Result<String, String> {
        let state = self.state.lock().await;
        state
            .sources
            .get(&(app.into(), id.into()))
            .map(|s| s.config_digest.clone())
            .ok_or_else(|| "source not configured".into())
    }

    /// Trusted host-only lifecycle port, called under the published-source approval
    /// fence. This never grants source execution. `demand` is derived from live
    /// consumers plus the explicit background lease, not accepted as authority.
    /// A background lease may be indefinite when explicitly authorized; restart is opt-in.
    pub async fn set_source_control(
        self: &Arc<Self>,
        app: &str,
        id: &str,
        expected_config_digest: &str,
        control: SourceControl,
    ) -> Result<(), String> {
        if control.background && control.expires_at_ms.is_some_and(|t| t <= now_ms()) {
            return Err("background lease expiry must be in the future".into());
        }
        if !control.background && control.resume_after_restart {
            return Err("restart resume requires a background lease".into());
        }
        self.control_expected(
            app,
            id,
            Some((expected_config_digest, control.background)),
            |s| {
                s.paused = control.paused;
                s.background = control.background;
                s.resume_after_restart = control.resume_after_restart;
                s.expires_at_ms = control.expires_at_ms;
                if control.background {
                    s.consumers.insert("__background_lease".into());
                } else {
                    s.consumers.remove("__background_lease");
                }
            },
        )
        .await
    }

    pub async fn set_demand(
        self: &Arc<Self>,
        app: &str,
        source: &str,
        consumer: &str,
        active: bool,
    ) -> Result<(), String> {
        if consumer.is_empty() || consumer.len() > 256 || consumer == "__background_lease" {
            return Err("invalid consumer".into());
        }
        self.control(app, source, |s| {
            if active {
                s.consumers.insert(consumer.to_owned());
            } else {
                s.consumers.remove(consumer);
            }
        })
        .await
    }

    pub async fn pause(
        self: &Arc<Self>,
        app: &str,
        source: &str,
        paused: bool,
    ) -> Result<(), String> {
        self.control(app, source, |s| s.paused = paused).await
    }

    /// Wake only an already authorized, demanded polling task. Concurrent requests
    /// share its next real read; cached snapshots never satisfy this operation.
    pub async fn refresh(self: &Arc<Self>, app: &str, id: &str) -> Result<SourceEvent, String> {
        self.begin_refresh(app, id, None).await?.wait().await
    }

    /// Admit under the caller's lifecycle fence, then wait without holding it.
    pub async fn begin_refresh(
        self: &Arc<Self>,
        app: &str,
        id: &str,
        expected_revision: Option<&str>,
    ) -> Result<RefreshWait, String> {
        let events = self.events.subscribe();
        let (generation, seq) = {
            let mut state = self.state.lock().await;
            if state.closed || state.suspended {
                return Err("automation service shut down".into());
            }
            if expected_revision.is_some_and(|revision| {
                state.revisions.get(app).map(String::as_str) != Some(revision)
            }) {
                return Err("source published revision changed".into());
            }
            let source = state
                .sources
                .get_mut(&(app.to_owned(), id.to_owned()))
                .ok_or("source not configured")?;
            if !source.granted
                || source.paused
                || !source_is_demanded(source)
                || source.task.is_none()
            {
                return Err("refresh requires granted, unpaused, demanded running source".into());
            }
            if !matches!(
                source.definition.driver,
                SourceDriver::Interval { .. } | SourceDriver::HttpPoll { .. }
            ) {
                return Err("refresh supports only interval and HTTP polling sources".into());
            }
            if !source.refresh_pending {
                source.refresh_pending = true;
                source.refresh.notify_one();
            }
            (source.generation.clone(), source.seq)
        };
        Ok(RefreshWait {
            events,
            service: Arc::downgrade(self),
            app: app.to_owned(),
            id: id.to_owned(),
            generation,
            seq,
        })
    }

    async fn control(
        self: &Arc<Self>,
        app: &str,
        id: &str,
        change: impl FnOnce(&mut Source),
    ) -> Result<(), String> {
        self.control_expected(app, id, None, change).await
    }

    async fn control_expected(
        self: &Arc<Self>,
        app: &str,
        id: &str,
        expected: Option<(&str, bool)>,
        change: impl FnOnce(&mut Source),
    ) -> Result<(), String> {
        let key = (app.to_owned(), id.to_owned());
        let mut state = self.state.lock().await;
        if state.closed {
            return Err("automation service shut down".into());
        }
        let suspended = state.suspended;
        let source = state.sources.get_mut(&key).ok_or("source not configured")?;
        if let Some((digest, enabling)) = expected {
            if source.config_digest != digest {
                return Err("source config digest changed".into());
            }
            if enabling && !source.granted {
                return Err("background lease requires source approval".into());
            }
        }
        // Cancel before changing authority; state lock fences all result forwarding.
        let was_granted = source.granted;
        change(source);

        if !source.granted || source.paused {
            stop(source).await;
        }
        let persisted = self.journal.grant_source(&GrantRecord {
            app_id: app.into(),
            source_id: id.into(),
            config_digest: source.config_digest.clone(),
            endpoint_grant: source
                .endpoint_grant
                .as_ref()
                .map(|g| serde_json::to_value(g).unwrap())
                .or_else(|| source.file_grant.as_ref().map(|g| json!({"file_grant":g}))),
            credential_refs: source
                .policy
                .credential_bindings
                .iter()
                .map(|b| b.reference.clone())
                .collect(),
            granted: source.granted,
        });
        if let Err(error) = persisted {
            source.granted = false;
            stop(source).await;
            source.error_class = Some("control_persistence_failed".into());
            self.emit(&key, source, SourceStatus::Error, None);
            return Err(error);
        }
        let persisted = self.journal.set_source_control(
            app,
            id,
            &SourceControl {
                paused: source.paused,
                demand: source_is_demanded(source),
                background: source.background,
                resume_after_restart: source.resume_after_restart,
                expires_at_ms: source.expires_at_ms,
                ..Default::default()
            },
        );
        if let Err(error) = persisted {
            source.granted = false;
            stop(source).await;
            source.error_class = Some("control_persistence_failed".into());
            self.emit(&key, source, SourceStatus::Error, None);
            return Err(error);
        }
        source.source_generation = self
            .journal
            .source_state(app, id)?
            .ok_or("missing source")?
            .source_generation;
        let _ = was_granted;
        let active = !suspended && source.granted && !source.paused && source_is_demanded(source);
        if !active {
            stop(source).await;
            self.emit(&key, source, SourceStatus::Passive, None);
        } else if source.task.is_none() {
            match source.definition.driver.clone() {
                SourceDriver::Manual
                | SourceDriver::Internal {
                    ..
                } => self.emit(&key, source, SourceStatus::Running, None),
                SourceDriver::Interval {
                    every_ms,
                }
                | SourceDriver::FileTail {
                    every_ms, ..
                }
                | SourceDriver::HttpPoll {
                    every_ms, ..
                } if every_ms >= 1000 => {
                    self.start_task(&key, source);
                }
                SourceDriver::Sse {
                    ..
                }
                | SourceDriver::Websocket {
                    ..
                } => self.start_task(&key, source),
                SourceDriver::Process {
                    ..
                } => self.emit(&key, source, SourceStatus::Running, None),
                _ => self.emit(&key, source, SourceStatus::Unsupported, None),
            }
        }
        Ok(())
    }

    fn start_task(self: &Arc<Self>, key: &(String, String), source: &mut Source) {
        source.generation = uuid::Uuid::new_v4().to_string();

        let driver = source.definition.driver.clone();
        let weak = Arc::downgrade(self);
        let generation = source.generation.clone();
        let endpoint_grant = source.endpoint_grant.clone();
        let refresh = source.refresh.clone();
        let policy = source.policy.clone();
        source.started_at = now_ms();
        let task_key = key.clone();
        let (cancel, mut cancelled) = watch::channel(false);
        let watchdog_service = weak.clone();
        let watchdog_key = task_key.clone();
        let watchdog_generation = generation.clone();
        let stale_after = policy.stale_after_ms;
        let task = tokio::spawn(async move {
            let watchdog = async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(stale_after.min(1000))).await;
                    let Some(owner) = watchdog_service.upgrade() else {
                        return;
                    };
                    let mut state = owner.state.lock().await;
                    let Some(source) = state.sources.get_mut(&watchdog_key) else {
                        return;
                    };
                    if !source_is_demanded(source) {
                        source.background = false;
                        source.consumers.remove("__background_lease");
                        source.task.take();
                        owner.emit(&watchdog_key, source, SourceStatus::Passive, None);
                        return;
                    }
                    if state_closed_source(source, &watchdog_generation) {
                        return;
                    }
                    if now_ms().saturating_sub(source.last_data_at.unwrap_or(source.started_at))
                        >= stale_after
                        && !matches!(source.status, SourceStatus::Stale)
                    {
                        owner.emit(&watchdog_key, source, SourceStatus::Stale, None);
                    }
                }
            };
            tokio::select! {
                _ = watchdog => {},
                _ = cancelled.changed() => {},
                _ = run_source(weak, task_key, generation, driver, endpoint_grant, refresh, policy) => {},
            }
        });
        source.task = Some((cancel, task));
        self.emit(key, source, SourceStatus::Running, None);
        if matches!(
            source.definition.driver,
            SourceDriver::Sse { .. } | SourceDriver::Websocket { .. }
        ) {
            self.emit(key, source, SourceStatus::Gap, Some(json!({"range":"unknown","reason":"collector_started","replay_capable":source.policy.replay_capable})));
        }
    }

    /// Host-only adapter: caller must authorize observation in the exec registry first.
    pub async fn publish_process_generation(
        &self,
        app: &str,
        id: &str,
        process_id: &str,
        expected_generation: &str,
        data: Value,
    ) -> Result<(), String> {
        let key = (app.to_owned(), id.to_owned());
        let mut state = self.state.lock().await;
        if state.closed || state.suspended {
            return Err("automation service shut down".into());
        }
        let source = state.sources.get_mut(&key).ok_or("source not configured")?;
        if source.generation != expected_generation {
            return Err("stale process source generation".into());
        }
        if !matches!(&source.definition.driver, SourceDriver::Process { process_id: expected, .. } if expected == process_id)
        {
            return Err("process source identity mismatch".into());
        }
        if !source.granted || source.paused || !source_is_demanded(source) {
            return Err("source is passive".into());
        }
        self.emit(&key, source, SourceStatus::Data, Some(data));
        if matches!(source.status, SourceStatus::Error) {
            return Err("source publish failed".into());
        }
        Ok(())
    }

    pub async fn publish(&self, app: &str, id: &str, data: Value) -> Result<(), String> {
        let key = (app.to_owned(), id.to_owned());
        let mut state = self.state.lock().await;
        if state.closed || state.suspended {
            return Err("automation service shut down".into());
        }
        let source = state.sources.get_mut(&key).ok_or("source not configured")?;
        if !matches!(source.definition.driver, SourceDriver::Manual) {
            return Err("not a manual source".into());
        }
        if !source.granted || source.paused || !source_is_demanded(source) {
            return Err("source is passive".into());
        }
        self.emit(&key, source, SourceStatus::Data, Some(data));
        if matches!(source.status, SourceStatus::Error) {
            return Err("source publish failed".into());
        }
        Ok(())
    }

    pub async fn publish_internal(&self, topic: &str, data: Value) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if state.closed || state.suspended {
            return Err("automation service shut down".into());
        }
        let mut failed = false;
        for (key, source) in &mut state.sources {
            if source.granted
                && !source.paused
                && source_is_demanded(source)
                && matches!(&source.definition.driver, SourceDriver::Internal { topic: t } if t == topic)
            {
                self.emit(key, source, SourceStatus::Data, Some(data.clone()));
                failed |= matches!(source.status, SourceStatus::Error);
            }
        }
        if failed {
            return Err("one or more internal source publishes failed".into());
        }
        Ok(())
    }

    /// Trusted host publication with an explicit occurrence identity. This port is
    /// not exposed as an App command; the origin owner cannot consume its handoff.
    pub async fn publish_internal_id(
        &self,
        topic: &str,
        data: Value,
        event_id: &str,
        origin: &str,
    ) -> Result<(), String> {
        if topic.is_empty()
            || topic.len() > 256
            || event_id.is_empty()
            || event_id.len() > 512
            || origin.is_empty()
            || origin.len() > 256
        {
            return Err("internal publication identity bounds".into());
        }
        let identity = crate::automation::digest_json(&json!([origin, topic, event_id]))?;
        let mut state = self.state.lock().await;
        if state.closed || state.suspended {
            return Err("automation service shut down".into());
        }
        let mut failed = false;
        for (key, source) in &mut state.sources {
            if key.0 != origin && source.granted && !source.paused && source_is_demanded(source)
                && matches!(&source.definition.driver, SourceDriver::Internal { topic: t } if t == topic)
                && source.definition.permissions.iter().any(|p| matches!(p, refact_apps_core::Permission::Internal { topic: t } if t == topic))
            {
                source.upstream_event_id = Some(identity.clone());
                failed |= self.emit_admitted(key, source, SourceStatus::Data, Some(data.clone())).is_err();
            }
        }
        if failed {
            Err("one or more internal source admissions failed".into())
        } else {
            Ok(())
        }
    }

    fn emit(
        &self,
        key: &(String, String),
        source: &mut Source,
        status: SourceStatus,
        data: Option<Value>,
    ) {
        let _ = self.emit_admitted(key, source, status, data);
    }

    fn emit_admitted(
        &self,
        key: &(String, String),
        source: &mut Source,
        status: SourceStatus,
        data: Option<Value>,
    ) -> Result<(), String> {
        if matches!(status, SourceStatus::Data) && !self.check_authority(key, source) {
            source.upstream_event_id = None;
            return Err("canonical authority denied".into());
        }
        let data = if matches!(status, SourceStatus::Data) {
            if let Some(spec) = &source.policy.transform {
                match apply_transform(spec, data.as_ref().unwrap_or(&Value::Null)) {
                    Ok(value) => Some(value),
                    Err(_) => {
                        source.error_class = Some("transform_limit_or_invalid".into());
                        source.status = SourceStatus::Error;
                        source.pending_checkpoint = None;
                        source.upstream_event_id = None;
                        self.emit(key, source, SourceStatus::Error, None);
                        return Err("source transform failed".into());
                    }
                }
            } else {
                data
            }
        } else {
            data
        };
        if matches!(status, SourceStatus::Data) {
            let request = IngestRequest {
                app_id: key.0.clone(),
                source_id: key.1.clone(),
                producer_id: source.config_digest.clone(),
                source_generation: source.source_generation.clone(),
                producer_epoch: source.policy.upstream_epoch.clone().unwrap_or_default(),
                event_id: source.upstream_event_id.take().unwrap_or_default(),
                cursor: source
                    .pending_checkpoint
                    .take()
                    .unwrap_or_else(|| source.checkpoint.clone()),
                source_config_digest: source.config_digest.clone(),
                data: if key.0 == "project-automation" {
                    json!({"app_id":key.0,"source_id":key.1,"channel":source.definition.channel,
                        "source_config_digest":source.config_digest,"data":data})
                } else {
                    data.clone().unwrap_or(Value::Null)
                },
                deliveries: if key.0 == "project-automation" {
                    vec!["automation_source_channels".into()]
                } else {
                    vec![]
                },
                at_ms: now_ms(),
            };
            match self.journal.ingest(&request) {
                Ok(crate::automation::IngestOutcome::Duplicate) => return Ok(()),
                Ok(_) => {
                    source.checkpoint = request.cursor;
                    source.error_class = None;
                }
                Err(_) => {
                    source.status = SourceStatus::Error;
                    source.error_class = Some("journal_commit".into());
                    self.emit(key, source, SourceStatus::Error, None);
                    return Err("source journal commit failed".into());
                }
            }
        }
        source.seq += 1;
        source.status = status.clone();
        if matches!(status, SourceStatus::Data) {
            source.last_data = data.clone();
            source.last_data_at = Some(now_ms());
        }
        let _ = self.events.send(SourceEvent {
            app_id: key.0.clone(),
            source_id: key.1.clone(),
            channel: source.definition.channel.clone(),
            generation: source.generation.clone(),
            seq: source.seq,
            status,
            data,
            health: health(source),
        });
        Ok(())
    }

    pub async fn snapshot(&self, app: &str) -> Vec<SourceEvent> {
        let state = self.state.lock().await;
        state
            .sources
            .iter()
            .filter(|((a, _), _)| a == app)
            .map(|(key, source)| SourceEvent {
                app_id: key.0.clone(),
                source_id: key.1.clone(),
                channel: source.definition.channel.clone(),
                generation: source.generation.clone(),
                seq: source.seq,
                status: source.status.clone(),
                data: source.last_data.clone(),
                health: health(source),
            })
            .collect()
    }

    /// A failed durable revoke is NOT success: retain a passive in-memory hold
    /// and return an error so the host cannot acknowledge removal/restart safety.
    pub async fn unconfigure(&self, app: &str) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let keys = state
            .sources
            .keys()
            .filter(|(a, _)| a == app)
            .cloned()
            .collect::<Vec<_>>();
        let mut failure = None;
        for key in keys {
            let source = state.sources.get_mut(&key).unwrap();
            stop(source).await;
            source.granted = false;
            source.paused = true;
            match self.journal.remove_source(&key.0, &key.1) {
                Ok(_) => {
                    self.emit(&key, source, SourceStatus::Passive, None);
                    state.sources.remove(&key);
                }
                Err(error) => {
                    source.error_class = Some("durable_revoke_failed".into());
                    self.emit(&key, source, SourceStatus::Error, None);
                    failure = Some(error);
                }
            }
        }
        if let Some(error) = failure {
            return Err(format!("durable source revocation failed: {error}"));
        }
        state.revisions.remove(app);
        Ok(())
    }

    pub async fn shutdown(&self) {
        let mut state = self.state.lock().await;
        state.closed = true;
        for source in state.sources.values_mut() {
            stop(source).await;
        }
    }
}

impl Drop for AutomationService {
    fn drop(&mut self) {
        for source in self.state.get_mut().sources.values_mut() {
            if let Some((cancel, task)) = source.task.take() {
                let _ = cancel.send(true);
                task.abort();
            }
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn health(source: &Source) -> Value {
    json!({"last_data_at_ms":source.last_data_at,"data_age_ms":source.last_data_at.map(|t|now_ms().saturating_sub(t)),
        "retry_at_ms":source.retry_at,"error_class":source.error_class,"background":source.background,"resume_after_restart":source.resume_after_restart,"expires_at_ms":source.expires_at_ms,
        "capabilities":{"replay":source.policy.replay_capable,"unique_event_ids":source.policy.unique_event_ids,"file_copytruncate_regrowth_detection":"unsupported","continuity":if source.policy.replay_capable {"replay_capable"} else {"best_effort"}},
        "checkpoint":source.checkpoint,"upstream_event_type":source.last_event_type})
}

async fn deliver(
    service: &Weak<AutomationService>,
    key: &(String, String),
    generation: &str,
    result: Result<Value, String>,
) -> bool {
    let Some(service) = service.upgrade() else {
        return false;
    };
    let mut state = service.state.lock().await;
    let Some(source) = state.sources.get_mut(key) else {
        return false;
    };
    if state_closed_source(source, generation) {
        return false;
    }
    // A credential identity/scope can be revoked while a network read is in
    // flight or a stream stays connected. Recheck its capability at the
    // forwarding fence; reconnect-only checks would publish revoked data.
    if !source.policy.credential_bindings.is_empty() && result.is_ok() {
        let raw = match &source.definition.driver {
            SourceDriver::HttpPoll {
                url, ..
            }
            | SourceDriver::Sse {
                url,
            }
            | SourceDriver::Websocket {
                url,
            } => Some(url),
            _ => None,
        };
        let allowed = raw
            .and_then(|raw| reqwest::Url::parse(raw).ok())
            .is_some_and(|url| {
                let origin = url.origin().ascii_serialization();
                let context = CredentialContext {
                    app_id: &key.0,
                    source_id: &key.1,
                    config_digest: &source.config_digest,
                    origin: &origin,
                };
                service
                    .credential_resolver
                    .lock()
                    .ok()
                    .is_some_and(|resolver| {
                        credentials::headers(
                            resolver.as_deref(),
                            &context,
                            &source.policy.credential_bindings,
                        )
                        .is_ok()
                    })
            });
        if !allowed {
            source.error_class = Some("credential_capability_denied".into());
            service.emit(
                key,
                source,
                SourceStatus::Error,
                Some(json!({"error":"credential capability denied"})),
            );
            return false;
        }
    }
    if source.refresh_pending {
        let _ = source.refresh.notified().now_or_never();
        source.refresh_pending = false;
    }
    match result {
        Ok(data) => {
            source.retry_at = None;
            source.error_class = None;
            service.emit(key, source, SourceStatus::Data, Some(data));
            if matches!(source.status, SourceStatus::Error) {
                return false;
            }
        }
        Err(error) => {
            source.error_class = Some(
                if error.contains("idle") {
                    "idle_timeout"
                } else if error.starts_with("retry_after_ms:") {
                    "upstream_retryable"
                } else {
                    "transport_or_upstream"
                }
                .into(),
            );
            service.emit(
                key,
                source,
                SourceStatus::Error,
                Some(json!({"error":"source read failed"})),
            );
        }
    }
    true
}

fn source_is_demanded(source: &Source) -> bool {
    source.consumers.iter().any(|c| c != "__background_lease")
        || (source.background && !source.expires_at_ms.is_some_and(|t| t <= now_ms()))
}

fn state_closed_source(source: &Source, generation: &str) -> bool {
    source.generation != generation
        || !source.granted
        || source.paused
        || !source_is_demanded(source)
}

async fn diagnostic(
    service: &Weak<AutomationService>,
    key: &(String, String),
    generation: &str,
    status: SourceStatus,
    detail: Value,
) {
    if let Some(service) = service.upgrade() {
        let mut state = service.state.lock().await;
        if let Some(source) = state.sources.get_mut(key) {
            if !state_closed_source(source, generation) {
                service.emit(key, source, status, Some(detail));
            }
        }
    }
}

async fn tail_once(
    service: &Weak<AutomationService>,
    key: &(String, String),
    generation: &str,
) -> Result<(), String> {
    tail_once_cooperative(service, key, generation, tokio::task::yield_now).await
}

async fn tail_once_cooperative<F, Fut>(
    service: &Weak<AutomationService>,
    key: &(String, String),
    generation: &str,
    checkpoint: F,
) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let owner = service.upgrade().ok_or("source unavailable")?;
    let (grant, cursor, timeout_ms) = {
        let state = owner.state.lock().await;
        let source = state.sources.get(key).ok_or("source unavailable")?;
        if state_closed_source(source, generation) {
            return Err("source stopped".into());
        }
        let grant = source
            .file_grant
            .clone()
            .ok_or("file capability forbidden")?;
        let SourceDriver::FileTail {
            path, ..
        } = &source.definition.driver
        else {
            return Err("not a file tail".into());
        };
        if path != &grant.path {
            return Err("file capability mismatch".into());
        }
        (grant, source.checkpoint.clone(), source.policy.timeout_ms)
    };
    let batch = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        file_tail::read_cooperative(&owner.file_root, &grant, cursor.as_deref(), checkpoint),
    )
    .await
    .map_err(|_| "file read timeout")??;
    let mut state = owner.state.lock().await;
    let source = state.sources.get_mut(key).ok_or("source unavailable")?;
    if state_closed_source(source, generation) || !owner.check_authority(key, source) {
        return Err("source stopped".into());
    }
    if batch.gap {
        owner.emit(
            key,
            source,
            SourceStatus::Gap,
            Some(json!({"range":"unknown","reason":"file_rotation_or_truncation"})),
        );
    }
    for (line, cursor) in batch.lines {
        source.pending_checkpoint = Some(Some(cursor));
        owner.emit(key, source, SourceStatus::Data, Some(json!({"line":line})));
        if source.error_class.is_some() {
            return Err("file checkpoint commit failed".into());
        }
    }
    Ok(())
}

async fn run_source(
    service: Weak<AutomationService>,
    key: (String, String),
    generation: String,
    driver: SourceDriver,
    grant: Option<EndpointGrant>,
    refresh: Arc<Notify>,
    policy: SourcePolicy,
) {
    let mut failures = 0u32;
    loop {
        // Bound external catalog edits at read admission, as well as at forwarding.
        let Some(owner) = service.upgrade() else {
            return;
        };
        {
            let mut state = owner.state.lock().await;
            let Some(source) = state.sources.get_mut(&key) else {
                return;
            };
            if state_closed_source(source, &generation) || !owner.check_authority(&key, source) {
                return;
            }
        }
        drop(owner);
        let result = match &driver {
            SourceDriver::FileTail {
                ..
            } => match tail_once(&service, &key, &generation).await {
                Ok(()) => Err("not_modified".into()),
                Err(error) => Err(error),
            },
            SourceDriver::Sse {
                url,
            } => stream_sse(&service, &key, &generation, url, grant.as_ref(), &policy).await,
            SourceDriver::Websocket {
                url,
            } => stream_ws(&service, &key, &generation, url, grant.as_ref(), &policy).await,
            SourceDriver::HttpPoll {
                url, ..
            } => {
                let headers = match service.upgrade() {
                    Some(owner) => owner.credential_headers(&key, &generation, url).await,
                    None => return,
                };
                match headers {
                    Ok(headers) => tokio::time::timeout(
                        Duration::from_millis(policy.timeout_ms),
                        poll_http_headers(url, grant.as_ref(), headers),
                    )
                    .await
                    .unwrap_or_else(|_| Err("read timeout".into())),
                    Err(error) => Err(error),
                }
            }
            _ => tokio::time::timeout(
                Duration::from_millis(policy.timeout_ms),
                read_driver(&driver, grant.as_ref()),
            )
            .await
            .unwrap_or_else(|_| Err("read timeout".into())),
        };
        let error = result.as_ref().err().cloned();
        let not_modified = error.as_deref() == Some("not_modified");
        if not_modified {
            // 304 validates transport only; it does not refresh observation age or
            // replace stale/error state with invented successful data.
            if let Some(owner) = service.upgrade() {
                let mut state = owner.state.lock().await;
                if let Some(source) = state.sources.get_mut(&key) {
                    if !state_closed_source(source, &generation) {
                        source.refresh_pending = false;
                    }
                }
            }
        } else if !deliver(&service, &key, &generation, result).await {
            break;
        }
        failures = if error.is_some() && !not_modified {
            failures.saturating_add(1)
        } else {
            0
        };
        let retry_after = error
            .as_deref()
            .and_then(|e| e.strip_prefix("retry_after_ms:"))
            .and_then(|v| v.parse().ok());
        let mut every_ms = if failures > 0 {
            policy.retry_delay(failures, retry_after)
        } else {
            match driver {
                SourceDriver::FileTail {
                    every_ms, ..
                } => every_ms,
                SourceDriver::Interval {
                    every_ms,
                }
                | SourceDriver::HttpPoll {
                    every_ms, ..
                } => every_ms,
                _ => policy.retry_base_ms,
            }
        };
        if matches!(
            driver,
            SourceDriver::Sse { .. } | SourceDriver::Websocket { .. }
        ) {
            diagnostic(&service, &key, &generation, SourceStatus::Gap,
                json!({"range":"unknown","reason":"connection_lost","replay_capable":policy.replay_capable})).await;
        }
        if let Some(owner) = service.upgrade() {
            let mut state = owner.state.lock().await;
            if let Some(source) = state.sources.get_mut(&key) {
                if matches!(driver, SourceDriver::Sse { .. }) {
                    every_ms = every_ms
                        .max(
                            source
                                .retry_at
                                .map(|t| t.saturating_sub(now_ms()))
                                .unwrap_or(0),
                        )
                        .min(policy.retry_cap_ms);
                }
                source.retry_at = Some(now_ms().saturating_add(every_ms));
                source.error_class = error.as_ref().map(|e| {
                    if e.contains("idle") {
                        "idle_timeout"
                    } else if e.contains("forbidden") {
                        "authorization"
                    } else if e.starts_with("retry_after_ms:") {
                        "upstream_retryable"
                    } else if e.contains("limit") {
                        "capacity"
                    } else {
                        "transport_or_upstream"
                    }
                    .into()
                });
            }
        }
        let sleep = tokio::time::sleep(Duration::from_millis(every_ms));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                _ = refresh.notified(), if failures == 0 => break,
                _ = tokio::time::sleep(Duration::from_millis(policy.stale_after_ms.min(1000))) => {
                    if let Some(owner) = service.upgrade() {
                        let mut state = owner.state.lock().await;
                        if let Some(source) = state.sources.get_mut(&key) {
                            if !state_closed_source(source, &generation) && now_ms().saturating_sub(source.last_data_at.unwrap_or(source.started_at)) >= policy.stale_after_ms && !matches!(source.status, SourceStatus::Stale) {
                                owner.emit(&key, source, SourceStatus::Stale, None);
                            }
                        }
                    } else { return; }
                }
            }
        }
    }
}

async fn stream_sse(
    service: &Weak<AutomationService>,
    key: &(String, String),
    generation: &str,
    raw: &str,
    grant: Option<&EndpointGrant>,
    policy: &SourcePolicy,
) -> Result<Value, String> {
    let (url, addresses) = endpoint(raw, false, grant).await?;
    let host = url.host_str().ok_or("missing host")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_millis(policy.timeout_ms))
        .read_timeout(Duration::from_millis(policy.idle_timeout_ms))
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|e| e.to_string())?;
    let headers = service
        .upgrade()
        .ok_or("source unavailable")?
        .credential_headers(key, generation, raw)
        .await?;
    let mut request = client
        .get(url)
        .headers(headers)
        .header("Accept", "text/event-stream");
    if policy.replay_capable {
        if let Some(owner) = service.upgrade() {
            if let Some(cursor) = owner.journal.checkpoint(&key.0, &key.1)? {
                request = request.header("Last-Event-ID", cursor);
            }
        }
    }
    let mut response = request.send().await.map_err(|e| e.to_string())?;
    if response.status().as_u16() == 429 || response.status().is_server_error() {
        let retry = retry_after_ms(response.headers(), policy.retry_cap_ms);
        return Err(format!("retry_after_ms:{retry}"));
    }
    response = response.error_for_status().map_err(|e| e.to_string())?;
    if response.status().is_redirection() {
        return Err("redirect forbidden".into());
    }
    let mut buffer = Vec::new();
    let mut last_event_id: Option<String> = None;
    let mut preceding_cr = false;
    while let Some(chunk) = tokio::time::timeout(
        Duration::from_millis(policy.idle_timeout_ms),
        response.chunk(),
    )
    .await
    .map_err(|_| "stream idle timeout")?
    .map_err(|e| e.to_string())?
    {
        // SSE accepts CR, LF and CRLF (including CRLF split across chunks).
        for byte in chunk {
            if byte == b'\n' && preceding_cr {
                preceding_cr = false;
                continue;
            }
            preceding_cr = byte == b'\r';
            buffer.push(if preceding_cr { b'\n' } else { byte });
        }
        while let Some((position, separator)) = buffer
            .windows(2)
            .position(|s| s == b"\n\n")
            .map(|p| (p, 2))
            .into_iter()
            .chain(
                buffer
                    .windows(4)
                    .position(|s| s == b"\r\n\r\n")
                    .map(|p| (p, 4)),
            )
            .min_by_key(|(p, _)| *p)
        {
            let frame = String::from_utf8(buffer.drain(..position + separator).collect())
                .map_err(|e| e.to_string())?;
            let frame = sse::parse(&frame);
            if let Some(id) = &frame.id {
                last_event_id = Some(id.clone());
            }
            if let Some(retry) = frame.retry_ms {
                if let Some(owner) = service.upgrade() {
                    let mut state = owner.state.lock().await;
                    if let Some(source) = state.sources.get_mut(key) {
                        if !state_closed_source(source, generation) {
                            source.retry_at =
                                Some(now_ms().saturating_add(retry.min(policy.retry_cap_ms)));
                        }
                    }
                }
            }
            if let Some(data) = frame.data {
                if let Some(owner) = service.upgrade() {
                    let mut state = owner.state.lock().await;
                    if let Some(source) = state.sources.get_mut(key) {
                        if !state_closed_source(source, generation) {
                            source.last_event_type = frame.event.clone();
                        }
                    }
                }
                if policy.replay_capable || policy.unique_event_ids {
                    if let Some(owner) = service.upgrade() {
                        let mut state = owner.state.lock().await;
                        if let Some(source) = state.sources.get_mut(key) {
                            if !state_closed_source(source, generation) {
                                if let Some(id) = last_event_id.clone() {
                                    if policy.replay_capable {
                                        source.pending_checkpoint =
                                            Some((!id.is_empty()).then_some(id.clone()));
                                    }
                                    source.upstream_event_id = if policy.unique_event_ids {
                                        frame.id.clone().filter(|id| !id.is_empty())
                                    } else {
                                        None
                                    };
                                }
                            }
                        }
                    }
                }
                if !deliver(
                    service,
                    key,
                    generation,
                    Ok(serde_json::from_str(&data).unwrap_or(Value::String(data))),
                )
                .await
                {
                    return Err("source stopped".into());
                }
            }
        }
    }
    Err("SSE closed".into())
}

async fn stream_ws(
    service: &Weak<AutomationService>,
    key: &(String, String),
    generation: &str,
    raw: &str,
    grant: Option<&EndpointGrant>,
    policy: &SourcePolicy,
) -> Result<Value, String> {
    let (url, addresses) = endpoint(raw, true, grant).await?;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use futures_util::SinkExt;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| "invalid websocket request")?;
    let headers = service
        .upgrade()
        .ok_or("source unavailable")?
        .credential_headers(key, generation, raw)
        .await?;
    request.headers_mut().extend(headers);
    if !policy.websocket_protocols.is_empty() {
        request.headers_mut().insert(
            "sec-websocket-protocol",
            policy
                .websocket_protocols
                .join(", ")
                .parse()
                .map_err(|_| "invalid websocket protocols")?,
        );
    }
    let socket = tokio::time::timeout(
        Duration::from_millis(policy.timeout_ms),
        tokio::net::TcpStream::connect(addresses.as_slice()),
    )
    .await
    .map_err(|_| "websocket connect timeout")?
    .map_err(|_| "websocket connect failed")?;
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None);
    let (mut stream, response) = tokio::time::timeout(
        Duration::from_millis(policy.timeout_ms),
        tokio_tungstenite::client_async_tls_with_config(request, socket, Some(config), None),
    )
    .await
    .map_err(|_| "websocket handshake timeout")?
    .map_err(|_| "websocket handshake failed")?;
    if !policy.websocket_protocols.is_empty()
        && !response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|p| {
                policy
                    .websocket_protocols
                    .iter()
                    .any(|allowed| allowed == p)
            })
    {
        return Err("websocket subprotocol negotiation failed".into());
    }
    if let Some(payload) = &policy.websocket_subscribe {
        stream
            .send(tokio_tungstenite::tungstenite::Message::Text(
                payload.to_string().into(),
            ))
            .await
            .map_err(|_| "websocket subscribe failed")?;
    }
    while let Some(message) =
        tokio::time::timeout(Duration::from_millis(policy.idle_timeout_ms), stream.next())
            .await
            .map_err(|_| "websocket idle timeout")?
    {
        let message = message.map_err(|e| e.to_string())?;
        if message.is_ping() {
            use futures_util::SinkExt;
            stream
                .send(tokio_tungstenite::tungstenite::Message::Pong(
                    message.into_data(),
                ))
                .await
                .map_err(|e| e.to_string())?;
            continue;
        }
        if message.is_close() {
            break;
        }
        if message.is_text() {
            let text = message.to_text().map_err(|e| e.to_string())?;
            if !deliver(
                service,
                key,
                generation,
                Ok(serde_json::from_str(text).unwrap_or(Value::String(text.to_owned()))),
            )
            .await
            {
                break;
            }
        }
    }
    Err("websocket closed".into())
}

async fn endpoint(
    raw: &str,
    websocket: bool,
    grant: Option<&EndpointGrant>,
) -> Result<(reqwest::Url, Vec<std::net::SocketAddr>), String> {
    let url = reqwest::Url::parse(raw).map_err(|e| e.to_string())?;
    let scheme_valid = if websocket {
        matches!(url.scheme(), "ws" | "wss")
    } else {
        matches!(url.scheme(), "http" | "https")
    };
    if !scheme_valid
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid source endpoint".into());
    }
    let host = url
        .host_str()
        .ok_or("endpoint has no host")?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = url.port_or_known_default().ok_or("endpoint has no port")?;
    let addresses = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?
    .collect::<Vec<_>>();
    if addresses.is_empty()
        || addresses.iter().any(|address| {
            !public_address(address.ip())
                && !grant.is_some_and(|grant| {
                    grant.origin == url.origin().ascii_serialization()
                        && grant.allowed_private_ips.contains(&address.ip())
                })
        })
    {
        return Err("non-public endpoint forbidden".into());
    }
    Ok((url, addresses))
}

async fn stop(source: &mut Source) {
    // Process/native observations also carry this fence even without a Tokio task.
    source.generation = uuid::Uuid::new_v4().to_string();
    source.refresh_pending = false;
    let _ = source.refresh.notified().now_or_never();
    if let Some((cancel, task)) = source.task.take() {
        let _ = cancel.send(true);
        // Aborting also cancels an in-flight read before waiting for its task.
        task.abort();
        let _ = task.await;
    }
}

async fn read_driver(
    driver: &SourceDriver,
    grant: Option<&EndpointGrant>,
) -> Result<Value, String> {
    match driver {
        SourceDriver::Interval {
            ..
        } => Ok(json!({"tick": true})),
        SourceDriver::HttpPoll {
            url, ..
        } => poll_http(url, grant).await,
        _ => Err("unsupported driver".into()),
    }
}

fn retry_after_ms(headers: &reqwest::header::HeaderMap, cap_ms: u64) -> u64 {
    let Some(raw) = headers.get("retry-after").and_then(|v| v.to_str().ok()) else {
        return 0;
    };
    if let Ok(seconds) = raw.parse::<u64>() {
        return seconds.saturating_mul(1000).min(cap_ms);
    }
    chrono::DateTime::parse_from_rfc2822(raw)
        .ok()
        .map(|date| {
            (date.timestamp_millis().max(0) as u64)
                .saturating_sub(now_ms())
                .min(cap_ms)
        })
        .unwrap_or(0)
}

async fn poll_http(raw: &str, grant: Option<&EndpointGrant>) -> Result<Value, String> {
    poll_http_headers(raw, grant, reqwest::header::HeaderMap::new()).await
}

async fn poll_http_headers(
    raw: &str,
    grant: Option<&EndpointGrant>,
    headers: reqwest::header::HeaderMap,
) -> Result<Value, String> {
    let (url, addresses) = endpoint(raw, false, grant).await?;
    let host = url.host_str().ok_or("missing host")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|_| "HTTP transport failed")?;
    if response.status().as_u16() == 304 {
        return Err("not_modified".into());
    }
    if response.status().as_u16() == 429 || response.status().is_server_error() {
        let retry = retry_after_ms(response.headers(), 300_000);
        return Err(format!("retry_after_ms:{retry}"));
    }
    response = response.error_for_status().map_err(|e| e.to_string())?;
    if response.status().is_redirection() {
        return Err("redirect forbidden".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn public_address(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 192 && b == 0)
        }
        // Conservatively allow only global-unicast IPv6, excluding transition/mapped ranges.
        std::net::IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && s[1] < 0x200)
                && !(s[0] == 0x2001 && s[1] == 0xdb8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixtureResolver;
    impl SourceCredentialResolver for FixtureResolver {
        fn resolve(
            &self,
            c: &CredentialContext<'_>,
            b: &CredentialBinding,
        ) -> Result<reqwest::header::HeaderValue, String> {
            if c.app_id != "app"
                || c.source_id != "secure"
                || b.expected_identity_revision.as_deref() != Some("identity-1")
            {
                return Err("denied".into());
            }
            Ok(reqwest::header::HeaderValue::from_static(
                "test-private-token",
            ))
        }
    }
    fn auth_policy() -> SourcePolicy {
        SourcePolicy {
            credential_bindings: vec![CredentialBinding {
                reference: "fixture".into(),
                header: "authorization".into(),
                scheme: Some("Bearer".into()),
                expected_identity_revision: Some("identity-1".into()),
            }],
            ..Default::default()
        }
    }
    #[tokio::test]
    async fn revoked_credential_cannot_forward_already_received_stream_data() {
        struct Revocable(std::sync::atomic::AtomicBool);
        impl SourceCredentialResolver for Revocable {
            fn resolve(
                &self,
                _: &CredentialContext<'_>,
                _: &CredentialBinding,
            ) -> Result<reqwest::header::HeaderValue, String> {
                if self.0.load(std::sync::atomic::Ordering::SeqCst) {
                    Ok(reqwest::header::HeaderValue::from_static("token"))
                } else {
                    Err("denied".into())
                }
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let owner = AutomationService::new(temp.path()).unwrap();
        owner
            .configure(
                "app",
                "ui".into(),
                vec![SourceDefinition {
                    source_id: "secure".into(),
                    channel: "values".into(),
                    driver: SourceDriver::Sse {
                        url: "https://example.com/events".into(),
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        owner
            .set_policy("app", "secure", auth_policy())
            .await
            .unwrap();
        let resolver = Arc::new(Revocable(std::sync::atomic::AtomicBool::new(true)));
        owner
            .set_credential_resolver(Some(resolver.clone()))
            .await
            .unwrap();
        owner.set_grant("app", "secure", true).await.unwrap();
        owner
            .set_demand("app", "secure", "test", true)
            .await
            .unwrap();
        let key = ("app".into(), "secure".into());
        let generation = owner.state.lock().await.sources[&key].generation.clone();
        assert!(
            deliver(
                &Arc::downgrade(&owner),
                &key,
                &generation,
                Ok(json!({"baseline":true})),
            )
            .await
        );
        assert_eq!(
            owner.journal.latest("app", "secure").unwrap(),
            Some(json!({"baseline":true}))
        );
        resolver.0.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(
            !deliver(
                &Arc::downgrade(&owner),
                &key,
                &generation,
                Ok(json!({"revoked":true}))
            )
            .await
        );
        assert_eq!(
            owner.journal.latest("app", "secure").unwrap(),
            Some(json!({"baseline":true}))
        );
        owner.shutdown().await;
    }

    #[tokio::test]
    async fn authenticated_http_never_journals_headers_and_replacement_revokes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let n = socket.read(&mut bytes).await.unwrap();
            assert!(String::from_utf8_lossy(&bytes[..n]).contains("Bearer test-private-token"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}").await.unwrap();
        });
        let owner = AutomationService::new(temp.path()).unwrap();
        owner
            .configure(
                "app",
                "ui".into(),
                vec![SourceDefinition {
                    source_id: "secure".into(),
                    channel: "values".into(),
                    driver: SourceDriver::HttpPoll {
                        url: format!("http://{address}/"),
                        every_ms: 60000,
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        owner
            .set_policy("app", "secure", auth_policy())
            .await
            .unwrap();
        owner
            .set_endpoint_grant(
                "app",
                "secure",
                Some(EndpointGrant {
                    origin: format!("http://{address}"),
                    allowed_private_ips: vec![address.ip()],
                }),
            )
            .await
            .unwrap();
        owner
            .set_credential_resolver(Some(Arc::new(FixtureResolver)))
            .await
            .unwrap();
        owner.set_grant("app", "secure", true).await.unwrap();
        let mut events = owner.subscribe();
        owner
            .set_demand("app", "secure", "confirmed_tool", true)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                assert!(!serde_json::to_string(&event)
                    .unwrap()
                    .contains("test-private-token"));
                if matches!(event.status, SourceStatus::Data) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            !serde_json::to_string(&owner.journal.list_sources().unwrap())
                .unwrap()
                .contains("test-private-token")
        );
        owner
            .set_credential_resolver(Some(Arc::new(FixtureResolver)))
            .await
            .unwrap();
        assert!(
            !owner
                .journal
                .source_state("app", "secure")
                .unwrap()
                .unwrap()
                .approved
        );
        owner.shutdown().await;
        server.await.unwrap();
    }
    #[tokio::test]
    async fn websocket_auth_subprotocol_subscription_and_reconnect() {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::{
            Message,
            handshake::server::{Request, Response},
        };
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for n in 1..=2 {
                let (socket, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_hdr_async(
                    socket,
                    |request: &Request, mut response: Response| {
                        assert_eq!(
                            request.headers()["authorization"],
                            "Bearer test-private-token"
                        );
                        assert_eq!(request.headers()["sec-websocket-protocol"], "metrics.v1");
                        response
                            .headers_mut()
                            .insert("sec-websocket-protocol", "metrics.v1".parse().unwrap());
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                assert_eq!(
                    ws.next().await.unwrap().unwrap().to_text().unwrap(),
                    "{\"subscribe\":\"metrics\"}"
                );
                ws.send(Message::Ping(vec![1, 2].into())).await.unwrap();
                assert!(ws.next().await.unwrap().unwrap().is_pong());
                ws.send(Message::Text(
                    json!({"n":n,"history":"ws-history".repeat(35000)})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
                ws.close(None).await.unwrap();
            }
        });
        let owner = AutomationService::new(temp.path()).unwrap();
        owner
            .configure(
                "app",
                "ui".into(),
                vec![SourceDefinition {
                    source_id: "secure".into(),
                    channel: "values".into(),
                    driver: SourceDriver::Websocket {
                        url: format!("ws://{address}/"),
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        let mut policy = auth_policy();
        policy.websocket_protocols = vec!["metrics.v1".into()];
        policy.websocket_subscribe = Some(json!({"subscribe":"metrics"}));
        policy.retry_base_ms = 100;
        policy.retry_cap_ms = 200;
        owner.set_policy("app", "secure", policy).await.unwrap();
        owner
            .set_endpoint_grant(
                "app",
                "secure",
                Some(EndpointGrant {
                    origin: format!("ws://{address}"),
                    allowed_private_ips: vec![address.ip()],
                }),
            )
            .await
            .unwrap();
        owner
            .set_credential_resolver(Some(Arc::new(FixtureResolver)))
            .await
            .unwrap();
        owner.set_grant("app", "secure", true).await.unwrap();
        let mut events = owner.subscribe();
        owner
            .set_demand("app", "secure", "confirmed_tool", true)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let e = events.recv().await.unwrap();
                if matches!(e.status, SourceStatus::Data)
                    && e.data == Some(json!({"n":2,"history":"ws-history".repeat(35000)}))
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        owner.shutdown().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn in_flight_file_read_releases_state_and_revoke_fences_stale_batch() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("log"),
            format!("{}\n", "history".repeat(50000)),
        )
        .unwrap();
        let owner = AutomationService::new(temp.path()).unwrap();
        owner
            .configure(
                "app",
                "ui".into(),
                vec![SourceDefinition {
                    source_id: "tail".into(),
                    channel: "values".into(),
                    driver: SourceDriver::FileTail {
                        path: "log".into(),
                        every_ms: 60000,
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        owner
            .set_file_grant(
                "app",
                "tail",
                Some(FileGrant {
                    path: "log".into(),
                    rotation: RotationPolicy::Follow,
                }),
            )
            .await
            .unwrap();
        owner.set_grant("app", "tail", true).await.unwrap();
        let key = ("app".into(), "tail".into());
        let generation = {
            let mut state = owner.state.lock().await;
            let source = state.sources.get_mut(&key).unwrap();
            source.consumers.insert("test".into());
            source.generation.clone()
        };
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let release = Arc::new(Notify::new());
        let release_read = release.clone();
        let weak = Arc::downgrade(&owner);
        let task = tokio::spawn(async move {
            let mut started_tx = Some(started_tx);
            let mut chunks = 0;
            tail_once_cooperative(&weak, &key, &generation, move || {
                chunks += 1;
                let started = if chunks == 2 { started_tx.take() } else { None };
                let release = release_read.clone();
                async move {
                    if let Some(started) = started {
                        started.send(()).unwrap();
                        release.notified().await;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            assert_eq!(owner.snapshot("app").await.len(), 1);
            owner.set_grant("app", "tail", false).await.unwrap();
            owner.shutdown().await;
        })
        .await
        .unwrap();
        release.notify_one();
        assert!(tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert_eq!(owner.journal.latest("app", "tail").unwrap(), None);
        assert_eq!(owner.journal.checkpoint("app", "tail").unwrap(), None);
    }

    #[tokio::test]
    async fn file_grant_and_committed_cursor_survive_service_restart() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("log"), b"first\npartial").unwrap();
        let owner = AutomationService::new(temp.path()).unwrap();
        owner
            .configure(
                "app",
                "ui".into(),
                vec![SourceDefinition {
                    source_id: "tail".into(),
                    channel: "lines".into(),
                    driver: SourceDriver::FileTail {
                        path: "log".into(),
                        every_ms: 1000,
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        owner
            .set_file_grant(
                "app",
                "tail",
                Some(FileGrant {
                    path: "log".into(),
                    rotation: RotationPolicy::Follow,
                }),
            )
            .await
            .unwrap();
        owner.set_grant("app", "tail", true).await.unwrap();
        let digest = owner.source_config_digest("app", "tail").await.unwrap();
        owner
            .set_source_control(
                "app",
                "tail",
                &digest,
                SourceControl {
                    background: true,
                    resume_after_restart: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if owner.journal.checkpoint("app", "tail").unwrap().is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let checkpoint = owner.journal.checkpoint("app", "tail").unwrap();
        owner.shutdown().await;
        drop(owner);
        std::fs::write(temp.path().join("log"), b"first\npartial completed\n").unwrap();
        let owner = AutomationService::new(temp.path()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if owner.journal.checkpoint("app", "tail").unwrap() != checkpoint {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            owner.journal.latest("app", "tail").unwrap(),
            Some(json!({"line":"partial completed"}))
        );
        owner.shutdown().await;
    }

    #[tokio::test]
    async fn unconfigure_tombstone_does_not_restore_or_reuse_old_approval() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new(temp.path()).unwrap();
        let definition = SourceDefinition {
            source_id: "input".into(),
            channel: "values".into(),
            driver: SourceDriver::Manual,
            permissions: vec![],
        };
        service
            .configure("app", "ui".into(), vec![definition.clone()])
            .await
            .unwrap();
        service.set_grant("app", "input", true).await.unwrap();
        service
            .set_demand("app", "input", "confirmed_tool", true)
            .await
            .unwrap();
        service.unconfigure("app").await.unwrap();
        service.shutdown().await;
        drop(service);
        let restored = AutomationService::new(temp.path()).unwrap();
        assert!(restored.snapshot("app").await.is_empty());
        restored
            .configure("app", "new-ui".into(), vec![definition])
            .await
            .unwrap();
        assert!(
            !restored
                .journal
                .source_state("app", "input")
                .unwrap()
                .unwrap()
                .approved
        );
        restored.shutdown().await;
    }

    #[tokio::test]
    async fn sse_stall_emits_stale_and_unknown_gap_without_inventing_checkpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n: heartbeat\n\n",
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(700)).await;
        });
        let service = AutomationService::new(temp.path()).unwrap();
        service
            .configure(
                "app",
                "ui".into(),
                vec![SourceDefinition {
                    source_id: "stream".into(),
                    channel: "values".into(),
                    driver: SourceDriver::Sse {
                        url: format!("http://{address}/events"),
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        service
            .set_policy(
                "app",
                "stream",
                SourcePolicy {
                    idle_timeout_ms: 300,
                    stale_after_ms: 100,
                    retry_base_ms: 1000,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        service
            .set_endpoint_grant(
                "app",
                "stream",
                Some(EndpointGrant {
                    origin: format!("http://{address}"),
                    allowed_private_ips: vec![address.ip()],
                }),
            )
            .await
            .unwrap();
        service.set_grant("app", "stream", true).await.unwrap();
        let mut events = service.subscribe();
        service
            .set_demand("app", "stream", "confirmed_tool", true)
            .await
            .unwrap();
        let (mut stale, mut gap) = (false, false);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !stale || !gap {
                let event = events.recv().await.unwrap();
                assert!(!matches!(event.status, SourceStatus::Data));
                stale |= matches!(event.status, SourceStatus::Stale);
                if matches!(event.status, SourceStatus::Gap) {
                    gap = true;
                    assert_eq!(event.data.unwrap()["range"], "unknown");
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(service.journal.checkpoint("app", "stream").unwrap(), None);
        assert_eq!(service.journal.latest("app", "stream").unwrap(), None);
        service.shutdown().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn sse_idle_reconnect_uses_only_committed_capability_cursor_and_revocation_fences() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            first.read(&mut request).await.unwrap();
            first.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nid: 41\nevent: update\ndata: {\"value\":41}\n\n").await.unwrap();
            let (mut second, _) = listener.accept().await.unwrap();
            let n = second.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..n]).to_ascii_lowercase();
            assert!(request.contains("last-event-id: 41"), "{request}");
            second.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nid: 41\ndata: {\"duplicate\":true}\n\nid: 42\ndata: {\"value\":42}\n\n").await.unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = second
                .write_all(b"id: 43\ndata: {\"revoked\":true}\n\n")
                .await;
        });
        let service = AutomationService::new(temp.path()).unwrap();
        service
            .configure(
                "app",
                "ui".into(),
                vec![SourceDefinition {
                    source_id: "stream".into(),
                    channel: "values".into(),
                    driver: SourceDriver::Sse {
                        url: format!("http://{address}/events"),
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        service
            .set_policy(
                "app",
                "stream",
                SourcePolicy {
                    idle_timeout_ms: 100,
                    retry_base_ms: 100,
                    retry_cap_ms: 200,
                    stale_after_ms: 100,
                    replay_capable: true,
                    unique_event_ids: true,
                    upstream_epoch: Some("upstream-run-1".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        service
            .set_endpoint_grant(
                "app",
                "stream",
                Some(EndpointGrant {
                    origin: format!("http://{address}"),
                    allowed_private_ips: vec![address.ip()],
                }),
            )
            .await
            .unwrap();
        service.set_grant("app", "stream", true).await.unwrap();
        let mut events = service.subscribe();
        service
            .set_demand("app", "stream", "confirmed_tool", true)
            .await
            .unwrap();
        let digest = service.source_config_digest("app", "stream").await.unwrap();
        service
            .set_source_control(
                "app",
                "stream",
                &digest,
                SourceControl {
                    background: true,
                    resume_after_restart: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(events.recv().await.unwrap().status, SourceStatus::Data) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            service
                .journal
                .checkpoint("app", "stream")
                .unwrap()
                .as_deref(),
            Some("41")
        );
        service.shutdown().await;
        drop(service);
        let restored = AutomationService::new(temp.path()).unwrap();
        let mut events = restored.subscribe();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if restored
                    .journal
                    .checkpoint("app", "stream")
                    .unwrap()
                    .as_deref()
                    == Some("42")
                {
                    break;
                }
                let event = events.recv().await.unwrap();
                assert_ne!(
                    event.data,
                    Some(json!({"duplicate":true})),
                    "stable upstream ID must not publish duplicate"
                );
            }
        })
        .await
        .unwrap();
        restored.set_grant("app", "stream", false).await.unwrap();
        server.await.unwrap();
        assert_eq!(
            restored.journal.latest("app", "stream").unwrap(),
            Some(json!({"value":42}))
        );
        restored.shutdown().await;
        drop(restored);
        let revoked = AutomationService::new(temp.path()).unwrap();
        assert!(
            !revoked
                .journal
                .source_state("app", "stream")
                .unwrap()
                .unwrap()
                .approved
        );
        assert!(matches!(
            revoked.snapshot("app").await[0].status,
            SourceStatus::Passive
        ));
        revoked.shutdown().await;
    }

    #[tokio::test]
    async fn ui_revision_preserves_approval_generation_and_source_specific_change_revokes() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new(temp.path()).unwrap();
        let definition = SourceDefinition {
            source_id: "input".into(),
            channel: "values".into(),
            driver: SourceDriver::Manual,
            permissions: vec![],
        };
        service
            .configure("app", "ui1".into(), vec![definition.clone()])
            .await
            .unwrap();
        service.set_grant("app", "input", true).await.unwrap();
        service
            .set_demand("app", "input", "confirmed_tool", true)
            .await
            .unwrap();
        service
            .publish("app", "input", json!({"n":1}))
            .await
            .unwrap();
        let generation = service.snapshot("app").await[0].generation.clone();
        service
            .configure("app", "ui2".into(), vec![definition.clone()])
            .await
            .unwrap();
        assert_eq!(service.snapshot("app").await[0].generation, generation);
        service
            .publish("app", "input", json!({"n":2}))
            .await
            .unwrap();
        service
            .configure(
                "app",
                "ui3".into(),
                vec![SourceDefinition {
                    channel: "new".into(),
                    ..definition
                }],
            )
            .await
            .unwrap();
        assert!(service.publish("app", "input", json!(3)).await.is_err());
        assert!(
            !service
                .journal
                .source_state("app", "input")
                .unwrap()
                .unwrap()
                .approved
        );
        service.shutdown().await;
    }

    #[tokio::test]
    async fn restart_restores_background_and_latest_but_not_view_demand_or_revoked_authority() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new(temp.path()).unwrap();
        let definitions = ["background", "view", "revoked"]
            .into_iter()
            .map(|id| SourceDefinition {
                source_id: id.into(),
                channel: "values".into(),
                driver: SourceDriver::Manual,
                permissions: vec![],
            })
            .collect();
        service
            .configure("app", "ui".into(), definitions)
            .await
            .unwrap();
        for id in ["background", "view", "revoked"] {
            service.set_grant("app", id, true).await.unwrap();
            service
                .set_demand(
                    "app",
                    id,
                    if id == "view" {
                        "viewer"
                    } else {
                        "confirmed_tool"
                    },
                    true,
                )
                .await
                .unwrap();
            if id != "view" {
                let digest = service.source_config_digest("app", id).await.unwrap();
                service
                    .set_source_control(
                        "app",
                        id,
                        &digest,
                        SourceControl {
                            background: true,
                            resume_after_restart: true,
                            expires_at_ms: Some(now_ms() + 60_000),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
            }
            service
                .publish("app", id, json!({"saved":id}))
                .await
                .unwrap();
        }
        service.set_grant("app", "revoked", false).await.unwrap();
        service.shutdown().await;
        drop(service);
        let restored = AutomationService::new(temp.path()).unwrap();
        assert_eq!(
            restored.journal.latest("app", "background").unwrap(),
            Some(json!({"saved":"background"}))
        );
        restored
            .publish("app", "background", json!({"after":true}))
            .await
            .unwrap();
        assert!(restored.publish("app", "view", json!(1)).await.is_err());
        assert!(restored.publish("app", "revoked", json!(1)).await.is_err());
        restored.shutdown().await;
    }

    #[tokio::test]
    async fn http_503_retry_after_and_304_never_fabricate_data() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let grant = EndpointGrant {
            origin: format!("http://{address}"),
            allowed_private_ips: vec![address.ip()],
        };
        let server = tokio::spawn(async move {
            for response in ["HTTP/1.1 503 Service Unavailable\r\nRetry-After: 20\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n"] {
                let (mut socket,_) = listener.accept().await.unwrap();
                let mut request=[0;2048]; socket.read(&mut request).await.unwrap(); socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let url = format!("http://{address}/data");
        assert_eq!(
            poll_http(&url, Some(&grant)).await.unwrap_err(),
            "retry_after_ms:20000"
        );
        assert_eq!(
            poll_http(&url, Some(&grant)).await.unwrap_err(),
            "not_modified"
        );
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "retry-after",
            (chrono::Utc::now() + chrono::Duration::seconds(30))
                .to_rfc2822()
                .parse()
                .unwrap(),
        );
        assert!((28_000..=30_000).contains(&retry_after_ms(&headers, 300_000)));
        let policy = SourcePolicy::default();
        assert!(policy.retry_delay(1, Some(20_000)) >= 20_000);
        assert_eq!(policy.retry_delay(30, None), policy.retry_cap_ms);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn refresh_requires_live_grants_and_coalesces_real_reads_without_cached_success() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new(temp.path()).unwrap();
        service
            .configure(
                "app",
                "revision".into(),
                vec![SourceDefinition {
                    source_id: "clock".into(),
                    channel: "ticks".into(),
                    driver: SourceDriver::Interval {
                        every_ms: 60_000,
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        assert!(service.refresh("app", "clock").await.is_err());
        service.set_grant("app", "clock", true).await.unwrap();
        assert!(service.refresh("app", "clock").await.is_err());
        let mut events = service.subscribe();
        service
            .set_demand("app", "clock", "viewer", true)
            .await
            .unwrap();
        let initial = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                if matches!(event.status, SourceStatus::Data) {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        let (a, b) = tokio::join!(
            service.refresh("app", "clock"),
            service.refresh("app", "clock")
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert!(a.seq > initial.seq, "cached tick cannot satisfy refresh");
        assert_eq!(a.seq, b.seq, "overlapping refreshes must share the read");
        assert_eq!(
            a.generation, initial.generation,
            "refresh must not restart source"
        );
        assert!(service
            .begin_refresh("app", "clock", Some("stale-revision"))
            .await
            .is_err());
        let pending = service
            .begin_refresh("app", "clock", Some("revision"))
            .await
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            service.set_grant("app", "clock", false),
        )
        .await
        .expect("revocation blocked behind refresh IO")
        .unwrap();
        assert!(
            pending.wait().await.is_err(),
            "revoked refresh exposed queued data"
        );
        service.pause("app", "clock", true).await.unwrap();
        assert!(service.refresh("app", "clock").await.is_err());
        assert_eq!(service.snapshot("app").await[0].data, a.data);
        service.shutdown().await;
    }

    #[tokio::test]
    async fn stale_process_read_cannot_publish_into_reconfigured_generation() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new(temp.path()).unwrap();
        let definition = SourceDefinition {
            source_id: "output".into(),
            channel: "logs".into(),
            driver: SourceDriver::Process {
                process_id: "exec_owned".into(),
                chat_id: None,
            },
            permissions: vec![],
        };
        service
            .configure("app", "first".into(), vec![definition.clone()])
            .await
            .unwrap();
        service.set_grant("app", "output", true).await.unwrap();
        service
            .set_demand("app", "output", "reader", true)
            .await
            .unwrap();
        let old = service.snapshot("app").await[0].generation.clone();
        service
            .configure(
                "app",
                "second".into(),
                vec![SourceDefinition {
                    channel: "changed".into(),
                    ..definition
                }],
            )
            .await
            .unwrap();
        service.set_grant("app", "output", true).await.unwrap();
        service
            .set_demand("app", "output", "reader", true)
            .await
            .unwrap();
        let current = service.snapshot("app").await[0].generation.clone();
        assert_ne!(old, current);
        assert!(
            service
                .publish_process_generation(
                    "app",
                    "output",
                    "exec_owned",
                    &old,
                    json!({"stale":true})
                )
                .await
                .is_err(),
            "old process read crossed source generation"
        );
        service
            .publish_process_generation(
                "app",
                "output",
                "exec_owned",
                &current,
                json!({"fresh":true}),
            )
            .await
            .unwrap();
        assert_eq!(
            service.snapshot("app").await[0].data,
            Some(json!({"fresh":true}))
        );
        service.shutdown().await;
    }

    #[tokio::test]
    async fn durable_revoke_and_internal_journal_failures_are_reported() {
        let temp = crate::test_support::tempdir();
        let service = AutomationService::new(temp.path()).unwrap();
        service
            .configure(
                "app",
                "r".into(),
                vec![SourceDefinition {
                    source_id: "input".into(),
                    channel: "values".into(),
                    driver: SourceDriver::Internal {
                        topic: "topic".into(),
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        service.set_grant("app", "input", true).await.unwrap();
        service
            .set_demand("app", "input", "view", true)
            .await
            .unwrap();
        let db = rusqlite::Connection::open(crate::automation::journal_path(temp.path()).unwrap())
            .unwrap();
        // Actual SQLite trigger failures, not a mock source/journal implementation.
        db.execute_batch("CREATE TRIGGER fail_ingest BEFORE INSERT ON events BEGIN SELECT RAISE(FAIL, 'fixture commit failure'); END;").unwrap();
        let mut events = service.subscribe();
        assert!(service
            .publish_internal("topic", json!({"value":1}))
            .await
            .is_err());
        let event = events.recv().await.unwrap();
        assert!(matches!(event.status, SourceStatus::Error));
        assert_eq!(event.health["error_class"], "journal_commit");
        assert!(service.journal.latest("app", "input").unwrap().is_none());
        db.execute_batch("CREATE TRIGGER fail_revoke BEFORE UPDATE ON sources BEGIN SELECT RAISE(FAIL, 'fixture revoke failure'); END;").unwrap();
        assert!(service.unconfigure("app").await.is_err());
        let snapshot = service.snapshot("app").await;
        assert_eq!(snapshot.len(), 1, "failed durable revoke must retain hold");
        assert_eq!(snapshot[0].health["error_class"], "durable_revoke_failed");
        assert!(
            service
                .journal
                .source_state("app", "input")
                .unwrap()
                .unwrap()
                .approved,
            "fixture proves durable authority was not silently claimed revoked"
        );
        assert!(service
            .publish_internal("topic", json!({"value":2}))
            .await
            .is_ok());
        db.execute_batch("DROP TRIGGER fail_revoke;").unwrap();
        service.unconfigure("app").await.unwrap();
        assert!(service.snapshot("app").await.is_empty());
        service.shutdown().await;
    }

    #[tokio::test]
    async fn lifecycle_lease_is_explicit_digest_bound_expiring_and_suspended() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new_suspended(temp.path()).unwrap();
        service
            .configure(
                "app",
                "r".into(),
                vec![SourceDefinition {
                    source_id: "input".into(),
                    channel: "values".into(),
                    driver: SourceDriver::Manual,
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        let digest = service.source_config_digest("app", "input").await.unwrap();
        let lease = SourceControl {
            background: true,
            resume_after_restart: true,
            expires_at_ms: Some(now_ms() + 1000),
            ..Default::default()
        };
        assert!(service
            .set_source_control("app", "input", &digest, lease.clone())
            .await
            .is_err());
        service.set_grant("app", "input", true).await.unwrap();
        assert!(service
            .set_source_control("app", "input", "stale", lease.clone())
            .await
            .is_err());
        service
            .set_demand("app", "input", "confirmed_tool", true)
            .await
            .unwrap();
        assert!(
            !service
                .journal
                .source_state("app", "input")
                .unwrap()
                .unwrap()
                .control
                .background
        );
        service
            .set_demand("app", "input", "confirmed_tool", false)
            .await
            .unwrap();
        service
            .set_source_control("app", "input", &digest, lease)
            .await
            .unwrap();
        assert!(service.publish("app", "input", json!(1)).await.is_err());
        service.activate().await.unwrap();
        service.publish("app", "input", json!(1)).await.unwrap();
        service
            .state
            .lock()
            .await
            .sources
            .get_mut(&("app".into(), "input".into()))
            .unwrap()
            .expires_at_ms = Some(now_ms() - 1);
        assert!(service.publish("app", "input", json!(2)).await.is_err());
        service.shutdown().await;
    }

    #[tokio::test]
    async fn internal_transform_failure_is_observable_and_not_success() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new(temp.path()).unwrap();
        service
            .configure(
                "app",
                "r".into(),
                vec![SourceDefinition {
                    source_id: "input".into(),
                    channel: "values".into(),
                    driver: SourceDriver::Internal {
                        topic: "topic".into(),
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        service
            .set_policy(
                "app",
                "input",
                SourcePolicy {
                    transform: Some(
                        serde_json::from_value(json!({"name":"object","version":1})).unwrap(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        service.set_grant("app", "input", true).await.unwrap();
        service
            .set_demand("app", "input", "view", true)
            .await
            .unwrap();
        let mut events = service.subscribe();
        assert!(service.publish_internal("topic", json!(7)).await.is_err());
        let event = events.recv().await.unwrap();
        assert!(matches!(event.status, SourceStatus::Error));
        assert_eq!(event.health["error_class"], "transform_limit_or_invalid");
        assert!(service.journal.latest("app", "input").unwrap().is_none());
        service.shutdown().await;
    }

    #[tokio::test]
    async fn sse_inherited_cursor_never_deduplicates_distinct_idless_dispatches() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for unique_event_ids in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 2048];
                socket.read(&mut request).await.unwrap();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nid: 7\ndata: {}\n\ndata: B\n\nid: 7\ndata: C\n\n", "A".repeat(350000)).as_bytes()).await.unwrap();
                tokio::time::sleep(Duration::from_millis(300)).await;
            });
            let service = AutomationService::new(temp.path()).unwrap();
            service
                .configure(
                    "app",
                    "r".into(),
                    vec![SourceDefinition {
                        source_id: "stream".into(),
                        channel: "values".into(),
                        driver: SourceDriver::Sse {
                            url: format!("http://{address}/events"),
                        },
                        permissions: vec![],
                    }],
                )
                .await
                .unwrap();
            service
                .set_policy(
                    "app",
                    "stream",
                    SourcePolicy {
                        replay_capable: true,
                        unique_event_ids,
                        upstream_epoch: Some("producer".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            service
                .set_endpoint_grant(
                    "app",
                    "stream",
                    Some(EndpointGrant {
                        origin: format!("http://{address}"),
                        allowed_private_ips: vec![address.ip()],
                    }),
                )
                .await
                .unwrap();
            service.set_grant("app", "stream", true).await.unwrap();
            let mut events = service.subscribe();
            service
                .set_demand("app", "stream", "view", true)
                .await
                .unwrap();
            let expected = if unique_event_ids {
                vec![json!("A".repeat(350000)), json!("B")]
            } else {
                vec![json!("A".repeat(350000)), json!("B"), json!("C")]
            };
            let mut observed = vec![];
            tokio::time::timeout(Duration::from_secs(2), async {
                while observed.len() < expected.len() {
                    let event = events.recv().await.unwrap();
                    if matches!(event.status, SourceStatus::Data) {
                        observed.push(event.data.unwrap());
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(observed, expected);
            assert_eq!(
                service
                    .journal
                    .checkpoint("app", "stream")
                    .unwrap()
                    .as_deref(),
                Some("7")
            );
            service.shutdown().await;
            server.await.unwrap();
        }
    }

    #[test]
    fn automation_lease_is_shared_with_project_owner() {
        let temp = crate::test_support::tempdir();
        let a = ProjectAutomationLease::acquire(temp.path()).unwrap();
        let b = ProjectAutomationLease::acquire(temp.path()).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn large_http_payload_reaches_supervisor_snapshot_and_survives_restart() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let temp = tempfile::tempdir().unwrap();
        let expected = json!({"history": (0..10000).map(|i| json!({"at":i,"value":"data".repeat(10)})).collect::<Vec<_>>()});
        let body = serde_json::to_vec(&expected).unwrap();
        assert!(body.len() > 299_000);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            for chunk in body.chunks(16384) {
                socket.write_all(chunk).await.unwrap();
            }
        });
        let service = AutomationService::new(temp.path()).unwrap();
        service
            .configure(
                "app",
                "ui".into(),
                vec![SourceDefinition {
                    source_id: "large".into(),
                    channel: "values".into(),
                    driver: SourceDriver::HttpPoll {
                        url: format!("http://{address}/history"),
                        every_ms: 60000,
                    },
                    permissions: vec![],
                }],
            )
            .await
            .unwrap();
        service
            .set_policy(
                "app",
                "large",
                SourcePolicy {
                    transform: Some(
                        serde_json::from_value(
                            json!({"name":"history","version":1,"select_fields":["history"]}),
                        )
                        .unwrap(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        service
            .set_endpoint_grant(
                "app",
                "large",
                Some(EndpointGrant {
                    origin: format!("http://{address}"),
                    allowed_private_ips: vec![address.ip()],
                }),
            )
            .await
            .unwrap();
        service.set_grant("app", "large", true).await.unwrap();
        let mut events = service.subscribe();
        service
            .set_demand("app", "large", "test", true)
            .await
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                if matches!(event.status, SourceStatus::Data) {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(event.data.as_ref(), Some(&expected));
        assert_eq!(
            service.journal.latest("app", "large").unwrap().as_ref(),
            Some(&expected)
        );
        assert_eq!(
            service.snapshot("app").await[0].data.as_ref(),
            Some(&expected)
        );
        service.shutdown().await;
        drop(service);
        server.await.unwrap();
        let restored = AutomationService::new(temp.path()).unwrap();
        assert_eq!(
            restored.journal.latest("app", "large").unwrap().as_ref(),
            Some(&expected)
        );
        assert_eq!(
            restored.snapshot("app").await[0].data.as_ref(),
            Some(&expected)
        );
        assert!(restored.publish("app", "large", expected).await.is_err());
        restored.shutdown().await;
    }

    #[tokio::test]
    async fn local_http_requires_exact_endpoint_ip_grant() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}/metrics");
        let grant = EndpointGrant {
            origin: format!("http://{address}"),
            allowed_private_ips: vec![address.ip()],
        };
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let count = socket.read(&mut request).await.unwrap();
            assert!(std::str::from_utf8(&request[..count])
                .unwrap()
                .starts_with("GET /metrics HTTP/1.1"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"step\":42}").await.unwrap();
        });
        assert!(
            poll_http(&url, None).await.is_err(),
            "private endpoint requires grant before connection"
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), poll_http(&url, Some(&grant)))
                .await
                .unwrap()
                .unwrap(),
            json!({"step":42})
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn declarations_do_not_grant_and_last_consumer_cancels_shared_interval() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new(temp.path()).unwrap();
        assert!(Arc::ptr_eq(
            &service,
            &AutomationService::new(temp.path().join(".")).unwrap()
        ));
        service
            .configure(
                "app",
                "revision".into(),
                vec![SourceDefinition {
                    source_id: "clock".into(),
                    channel: "ticks".into(),
                    driver: SourceDriver::Interval {
                        every_ms: 1000,
                    },
                    permissions: vec![refact_apps_core::Permission::Process],
                }],
            )
            .await
            .unwrap();
        service
            .set_demand("app", "clock", "chat-a", true)
            .await
            .unwrap();
        {
            let state = service.state.lock().await;
            assert!(
                state.sources[&("app".into(), "clock".into())]
                    .task
                    .is_none(),
                "declarations and demand must not start execution"
            );
        }
        service.set_grant("app", "clock", true).await.unwrap();
        service
            .set_demand("app", "clock", "chat-b", true)
            .await
            .unwrap();
        service
            .set_demand("app", "clock", "chat-a", false)
            .await
            .unwrap();
        {
            let state = service.state.lock().await;
            let task = &state.sources[&("app".into(), "clock".into())].task;
            assert!(task.is_some(), "second chat retains shared source");
        }
        service
            .set_demand("app", "clock", "chat-b", false)
            .await
            .unwrap();
        {
            let state = service.state.lock().await;
            assert!(
                state.sources[&("app".into(), "clock".into())]
                    .task
                    .is_none(),
                "last demand cancels and joins task"
            );
        }
        service.shutdown().await;
        assert!(service.set_grant("app", "clock", true).await.is_err());
    }
}
