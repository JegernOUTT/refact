use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Weak},
};
use refact_apps_core::{AppStore, Permission, SourceDefinition, SourceDriver};
use refact_privacy::FileRecord;
use refact_scheduler_core::sources::AutomationService;
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};
use crate::{
    exec::{ExecProcessId, ExecProcessSnapshot},
    global_context::GlobalContext,
};

const MAX_TARGETS: usize = 1024;
const MAX_RECORDS: usize = 16384;
const MAX_TEXT: usize = 32 * 1024;

struct Target {
    app: String,
    source: String,
    revision: String,
    config_digest: String,
    generation: String,
    process: ExecProcessId,
    caller: String,
    checkout: PathBuf,
    cursor: u64,
    terminal_sent: bool,
}
#[derive(Default)]
struct Provenance {
    records: HashSet<FileRecord>,
    processes: HashMap<ExecProcessId, (String, PathBuf)>,
    failure: Option<String>,
    inherited_incomplete: bool,
    marker_loaded: bool,
    slug: Option<String>,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableProvenance {
    schema_version: u32,
    incomplete: bool,
    files: Vec<FileRecord>,
}
#[derive(Default)]
struct State {
    targets: HashMap<(String, String), Target>,
    provenance: HashMap<String, Provenance>,
    by_process: HashMap<ExecProcessId, HashSet<(String, String)>>,
}

pub struct ProcessSourceAdapter {
    gcx: Weak<GlobalContext>,
    root: PathBuf,
    service: Arc<AutomationService>,
    state: Mutex<State>,
    cancel: watch::Sender<bool>,
    wake: tokio::sync::Notify,
    started: std::sync::atomic::AtomicBool,
    store: tokio::sync::OnceCell<AppStore>,
}
impl ProcessSourceAdapter {
    pub fn new(
        gcx: Arc<GlobalContext>,
        root: PathBuf,
        service: Arc<AutomationService>,
    ) -> Arc<Self> {
        let (cancel, _) = watch::channel(false);
        Arc::new(Self {
            gcx: Arc::downgrade(&gcx),
            root,
            service,
            state: Mutex::new(State::default()),
            cancel,
            wake: tokio::sync::Notify::new(),
            started: std::sync::atomic::AtomicBool::new(false),
            store: tokio::sync::OnceCell::new(),
        })
    }

    /// The runtime owns and joins the returned task. Only confirmed tool code calls sync.
    pub fn start(self: &Arc<Self>) -> JoinHandle<()> {
        let this = self.clone();
        let mut cancel = this.cancel.subscribe();
        // Subscribe before returning so initial output cannot fall between registration and startup.
        let Some(gcx) = this.gcx.upgrade() else {
            return tokio::spawn(async {});
        };
        let mut output = gcx.exec_registry.subscribe_output();
        let mut completion = gcx.exec_registry.subscribe_completion();
        let mut spawn = gcx.exec_registry.subscribe_spawn();
        let already_started = this.started.swap(true, std::sync::atomic::Ordering::AcqRel);
        tokio::spawn(async move {
            if already_started {
                return;
            }
            let mut sweep = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                if *cancel.borrow() {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = cancel.changed() => break,
                    event = output.recv() => match event {
                        Ok(chunk) => this.pump(Some(&chunk.process_id), false).await,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => this.pump(None, true).await,
                        Err(_) => break,
                    },
                    event = completion.recv() => match event {
                        Ok(event) => this.pump(Some(&event.process_id), false).await,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => this.pump(None, true).await,
                        Err(_) => break,
                    },
                    event = spawn.recv() => match event {
                        Ok(event) => this.pump(Some(&event.process_id), false).await,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => this.pump(None, true).await,
                        Err(_) => break,
                    },
                    _ = this.wake.notified() => this.pump(None, false).await,
                    _ = sweep.tick() => this.pump(None, false).await,
                }
            }
        })
    }

    pub async fn sync(
        &self,
        app: &str,
        revision: &str,
        defs: &[SourceDefinition],
        approved_caller_chat: &str,
    ) -> Result<(), String> {
        if *self.cancel.borrow() {
            return Err("process adapter shut down".into());
        }
        if approved_caller_chat.is_empty() {
            return Err("process source requires an owning caller chat".into());
        }
        let snapshots = self.service.snapshot(app).await;
        let mut targets = Vec::new();
        for def in defs {
            let SourceDriver::Process {
                process_id,
                chat_id,
            } = &def.driver
            else {
                continue;
            };
            if !def
                .permissions
                .iter()
                .any(|p| matches!(p, Permission::Process))
            {
                return Err("process source requires process permission".into());
            }
            if chat_id
                .as_deref()
                .is_some_and(|chat| chat != approved_caller_chat)
            {
                return Err("manifest chat identity is not process authority".into());
            }
            let process = ExecProcessId(process_id.clone());
            let checkout = self.owner_checkout(approved_caller_chat).await?;
            self.authorize(&process, approved_caller_chat, &checkout)
                .await?;
            let generation = snapshots
                .iter()
                .find(|s| s.source_id == def.source_id)
                .ok_or("process source must first be configured")?
                .generation
                .clone();
            targets.push(Target {
                app: app.into(),
                source: def.source_id.clone(),
                revision: revision.into(),
                config_digest: self
                    .service
                    .source_config_digest(app, &def.source_id)
                    .await?,
                generation,
                process,
                caller: approved_caller_chat.into(),
                checkout,
                cursor: 0,
                terminal_sent: false,
            });
        }
        let mut state = self.state.lock().await;
        if state.provenance.len() >= 256 && !state.provenance.contains_key(app) {
            return Err("process provenance app capacity reached".into());
        }
        if state
            .provenance
            .get(app)
            .is_some_and(|p| p.processes.len() + targets.len() > MAX_TARGETS)
        {
            return Err("process provenance identity capacity reached".into());
        }
        if state.targets.len() + targets.len() > MAX_TARGETS {
            return Err("process source capacity reached".into());
        }
        state
            .targets
            .retain(|(a, _), t| a != app || t.revision == revision);
        for target in targets {
            let key = (target.app.clone(), target.source.clone());
            let unchanged = state.targets.get(&key).is_some_and(|old| {
                old.revision == target.revision
                    && old.config_digest == target.config_digest
                    && old.generation == target.generation
                    && old.process == target.process
                    && old.caller == target.caller
                    && old.checkout == target.checkout
            });
            state
                .provenance
                .entry(app.into())
                .or_default()
                .processes
                .insert(
                    target.process.clone(),
                    (target.caller.clone(), target.checkout.clone()),
                );
            if !unchanged {
                state.targets.insert(key, target);
            }
        }
        state.by_process.clear();
        let indexed: Vec<_> = state
            .targets
            .iter()
            .map(|(k, t)| (t.process.clone(), k.clone()))
            .collect();
        for (process, key) in indexed {
            state.by_process.entry(process).or_default().insert(key);
        }
        self.wake.notify_one();
        Ok(())
    }

    async fn owner_checkout(&self, caller: &str) -> Result<PathBuf, String> {
        let gcx = self.gcx.upgrade().ok_or("process runtime unavailable")?;
        let session = gcx
            .chat_sessions
            .read()
            .await
            .get(caller)
            .cloned()
            .ok_or("process owner session unavailable")?;
        let (source, checkout, worktree_id) = {
            let session = session
                .try_lock()
                .map_err(|_| "process owner session busy")?;
            let source = crate::coordination::scope::session_source_root(&session)
                .ok_or("process owner project unavailable")?;
            let checkout = session
                .thread
                .worktree
                .as_ref()
                .map(|w| w.root.clone())
                .unwrap_or_else(|| source.clone());
            (
                source,
                checkout,
                session.thread.worktree.as_ref().map(|w| w.id.clone()),
            )
        };
        let source =
            crate::coordination::roots::resolve_source_workspace_root(gcx.clone(), source.to_str())
                .await
                .map_err(|_| "process owner project unavailable")?;
        if crate::exec::types::normalize_workspace_path(&source)
            != crate::exec::types::normalize_workspace_path(&self.root)
        {
            return Err("process owner project changed".into());
        }
        let checkout = tokio::fs::canonicalize(checkout)
            .await
            .map_err(|_| "process owner checkout unavailable")?;
        if let Some(id) = worktree_id {
            let roots = crate::coordination::roots::load_project_roots(gcx, source).await;
            let registered = roots
                .get(&id)
                .and_then(|r| r.path.as_ref())
                .ok_or("process owner worktree not registered")?;
            if tokio::fs::canonicalize(registered)
                .await
                .map_err(|_| "process owner checkout unavailable")?
                != checkout
            {
                return Err("process owner checkout not registered".into());
            }
        }
        Ok(crate::exec::types::normalize_workspace_path(&checkout))
    }

    async fn authorize(
        &self,
        process: &ExecProcessId,
        caller: &str,
        checkout: &std::path::Path,
    ) -> Result<ExecProcessSnapshot, String> {
        if tokio::fs::canonicalize(&self.root)
            .await
            .map_err(|_| "process source root unavailable")?
            != self.root
        {
            return Err("process source root changed".into());
        }
        if self.owner_checkout(caller).await? != checkout {
            return Err("process owner checkout changed".into());
        }
        let gcx = self.gcx.upgrade().ok_or("process runtime unavailable")?;
        let snapshot = gcx
            .exec_registry
            .authorize_process_access(process, caller, Some(checkout))
            .await?;
        if snapshot.meta.owner.chat_id.as_deref() != Some(caller)
            || snapshot
                .meta
                .owner
                .workspace
                .as_ref()
                .map(|p| crate::exec::types::normalize_workspace_path(p))
                != Some(crate::exec::types::normalize_workspace_path(checkout))
        {
            return Err("process source owner or checkout mismatch".into());
        }
        Ok(snapshot)
    }

    async fn capture(
        &self,
        provenance: &mut Provenance,
        process: &ExecProcessId,
        caller: &str,
        checkout: &std::path::Path,
    ) -> Result<(), String> {
        if let Some(reason) = &provenance.failure {
            return Err(reason.clone());
        }
        self.authorize(process, caller, checkout).await?;
        let gcx = self.gcx.upgrade().ok_or("process runtime unavailable")?;
        let reader = gcx
            .exec_registry
            .observation_reader(process)
            .await
            .ok_or("process observation unavailable")?;
        let status = reader.status();
        let (access, complete) = match status {
            refact_exec::ObservationStatus::Observed(access) => (access, true),
            refact_exec::ObservationStatus::Pending(access)
            | refact_exec::ObservationStatus::Incomplete(access) => (access, false),
            refact_exec::ObservationStatus::Unavailable(_) => {
                return Err("process observation unavailable".into())
            }
        };
        let records = crate::privacy::records::observed_file_records(&gcx, access.reads)?;
        provenance.records.extend(records);
        // Static classification alone misses files tainted by earlier reads in the owner session.
        let session = gcx
            .chat_sessions
            .read()
            .await
            .get(caller)
            .cloned()
            .ok_or("process owner privacy session unavailable")?;
        let session = session
            .try_lock()
            .map_err(|_| "process owner privacy session busy")?;
        let derived = session
            .derived_privacy_zones
            .read()
            .map_err(|_| "process derived privacy unavailable")?;
        provenance
            .records
            .extend(derived.iter().map(|(path, zone)| FileRecord {
                path: refact_core::chat_types::normalize_file_name(
                    path.to_string_lossy().into_owned(),
                ),
                zone: zone.zone.clone(),
                attribution: refact_privacy::Attribution::Observed,
            }));
        if provenance.records.len() > MAX_RECORDS {
            provenance.failure = Some("process provenance capacity exceeded".into());
            return Err("process provenance capacity exceeded".into());
        }
        if !complete {
            return Err("process observation incomplete".into());
        }
        Ok(())
    }

    async fn provenance_store(&self) -> Result<&AppStore, String> {
        self.store
            .get_or_try_init(|| AppStore::open(&self.root))
            .await
    }

    async fn load_marker(&self, app: &str, provenance: &mut Provenance) -> Result<(), String> {
        if provenance.marker_loaded {
            return Ok(());
        }
        let store = self.provenance_store().await?;
        let id = uuid::Uuid::parse_str(app).map_err(|_| "invalid process source app identity")?;
        let record = store.get_by_id(id).await?;
        provenance.slug = Some(record.slug.clone());
        let marker = store.read_provenance(&record.slug).await?;
        if !marker.value.is_null() {
            let marker: DurableProvenance =
                serde_json::from_value(marker.value).map_err(|_| "invalid process provenance")?;
            if marker.schema_version != 1 {
                return Err("unsupported process provenance".into());
            }
            if marker.files.len() > MAX_RECORDS {
                return Err("process provenance capacity exceeded".into());
            }
            provenance.inherited_incomplete = marker.incomplete;
            provenance.records.extend(marker.files);
        }
        provenance.marker_loaded = true;
        Ok(())
    }

    async fn persist_marker(&self, app: &str, provenance: &mut Provenance) -> Result<(), String> {
        self.load_marker(app, provenance).await?;
        let mut incomplete = provenance.inherited_incomplete || provenance.failure.is_some();
        for (process, (caller, checkout)) in provenance.processes.clone() {
            if self
                .capture(provenance, &process, &caller, &checkout)
                .await
                .is_err()
            {
                incomplete = true;
            }
        }
        let store = self.provenance_store().await?;
        let slug = provenance
            .slug
            .as_deref()
            .ok_or("process provenance identity unavailable")?;
        let record = store.published(slug).await?;
        if record.manifest.app_id.to_string() != app {
            return Err("process provenance app identity changed".into());
        }
        let old = store.read_provenance(slug).await?;
        // A second writer cannot clear a marker created outside our retained observation chain.
        let mut files = provenance.records.clone();
        if !old.value.is_null() {
            let prior: DurableProvenance = serde_json::from_value(old.value.clone())
                .map_err(|_| "invalid process provenance")?;
            if prior.schema_version != 1 {
                return Err("unsupported process provenance".into());
            }
            files.extend(prior.files);
        }
        if files.len() > MAX_RECORDS {
            return Err("process provenance capacity exceeded".into());
        }
        let mut ordered_files: Vec<_> = files.iter().cloned().collect();
        ordered_files.sort_by(|a, b| (&a.path, &a.zone).cmp(&(&b.path, &b.zone)));
        let value = serde_json::to_value(DurableProvenance {
            schema_version: 1,
            incomplete,
            files: ordered_files,
        })
        .map_err(|e| e.to_string())?;
        if old.value != value {
            store.write_provenance(slug, old.revision, value).await?;
        }
        provenance.records = files;
        Ok(())
    }

    pub async fn known_records(&self, app: &str) -> Result<Vec<FileRecord>, String> {
        let mut state = self.state.lock().await;
        let provenance = state.provenance.entry(app.into()).or_default();
        self.load_marker(app, provenance).await?;
        if provenance.inherited_incomplete {
            return Err("persisted process provenance incomplete".into());
        }
        if let Some(reason) = &provenance.failure {
            return Err(reason.clone());
        }
        let processes = provenance.processes.clone();
        for (process, (caller, checkout)) in processes {
            self.capture(provenance, &process, &caller, &checkout)
                .await?;
        }
        Ok(provenance.records.iter().cloned().collect())
    }

    async fn pump(&self, process: Option<&ExecProcessId>, lagged: bool) {
        let mut state = self.state.lock().await;
        let keys: Vec<_> = match process {
            Some(process) => state
                .by_process
                .get(process)
                .map(|keys| keys.iter().cloned().collect())
                .unwrap_or_default(),
            None => state.targets.keys().cloned().collect(),
        };
        for key in keys {
            let Some(mut target) = state.targets.remove(&key) else {
                continue;
            };
            if *self.cancel.borrow() {
                state.targets.insert(key, target);
                break;
            }
            let result = self.read_target(&mut state, &mut target, lagged).await;
            if let Err(reason) = result {
                state
                    .provenance
                    .entry(target.app.clone())
                    .or_default()
                    .failure = Some(reason);
            }
            state.targets.insert(key, target);
        }
    }

    async fn read_target(
        &self,
        state: &mut State,
        target: &mut Target,
        lagged: bool,
    ) -> Result<(), String> {
        let snapshot = self
            .authorize(&target.process, &target.caller, &target.checkout)
            .await?;
        let current = self.service.snapshot(&target.app).await;
        let Some(current) = current.iter().find(|s| s.source_id == target.source) else {
            return Ok(());
        };
        // Read the digest AFTER the generation snapshot: a concurrent config
        // change must either fail this check or invalidate publication's generation.
        // Only lifecycle churn of this exact trusted registration may rebind;
        // a changed definition/policy needs a fresh trusted sync.
        if self
            .service
            .source_config_digest(&target.app, &target.source)
            .await?
            != target.config_digest
        {
            return Ok(());
        }
        target.generation = current.generation.clone();
        let gcx = self.gcx.upgrade().ok_or("process runtime unavailable")?;
        let read = gcx
            .exec_registry
            .read(&target.process, target.cursor, Some(64))
            .await;
        if !read.found {
            return Err("process expired".into());
        }
        let provenance = state.provenance.entry(target.app.clone()).or_default();
        // Retain references even for unobserved local-only bytes; expiry must never erase taint.
        provenance.processes.insert(
            target.process.clone(),
            (target.caller.clone(), target.checkout.clone()),
        );
        let observation = self
            .capture(
                provenance,
                &target.process,
                &target.caller,
                &target.checkout,
            )
            .await;
        let observation_status = match &observation {
            Ok(()) => "observed",
            Err(e) if e == "process observation incomplete" => "incomplete",
            _ => "unavailable",
        };
        if observation_status == "unavailable" {
            provenance.failure = Some("process observation unavailable".into());
        }
        self.persist_marker(&target.app, provenance).await?;
        let mut cursor = target.cursor;
        for chunk in read.chunks {
            let gap = lagged || chunk.seq != cursor;
            let (text, truncated) = bounded_text(&chunk.text);
            let data = serde_json::json!({"process_id":target.process.as_str(),"seq":chunk.seq,
                "stream":chunk.stream,"text":text,"gap":gap,"truncated":truncated,
                "timestamp_ms":chunk.timestamp_ms,"status":snapshot.status,
                "privacy_observation":observation_status});
            if self
                .service
                .publish_process_generation(
                    &target.app,
                    &target.source,
                    target.process.as_str(),
                    &target.generation,
                    data,
                )
                .await
                .is_err()
            {
                return Ok(());
            }
            cursor = chunk.seq.saturating_add(1);
            target.cursor = cursor;
        }
        if snapshot.status.is_terminal()
            && target.cursor >= read.latest_seq
            && !target.terminal_sent
        {
            let data = serde_json::json!({"process_id":target.process.as_str(),"seq":target.cursor,
                "stream":"status","text":"","status":snapshot.status,"privacy_observation":observation_status,"gap":lagged});
            if self
                .service
                .publish_process_generation(
                    &target.app,
                    &target.source,
                    target.process.as_str(),
                    &target.generation,
                    data,
                )
                .await
                .is_ok()
            {
                target.terminal_sent = true;
            }
        }
        Ok(())
    }

    pub async fn registered_source_ids(&self, app: &str) -> Vec<String> {
        self.state
            .lock()
            .await
            .targets
            .keys()
            .filter(|(a, _)| a == app)
            .map(|(_, s)| s.clone())
            .collect()
    }

    pub async fn revoke(&self, app: &str, source: &str) {
        let mut state = self.state.lock().await;
        state.targets.remove(&(app.into(), source.into()));
        for targets in state.by_process.values_mut() {
            targets.remove(&(app.into(), source.into()));
        }
        // Do not erase provenance: running views and saved state can still retain delivered bytes.
    }

    pub async fn unconfigure(&self, app: &str) {
        let mut state = self.state.lock().await;
        state.targets.retain(|(a, _), _| a != app);
        for targets in state.by_process.values_mut() {
            targets.retain(|(a, _)| a != app);
        }
        // App state or an existing view may still contain old bytes. Keep provenance until shutdown.
    }
    pub async fn shutdown(&self) {
        self.cancel.send_replace(true);
        self.wake.notify_waiters();
        self.state.lock().await.targets.clear();
    }
}

fn bounded_text(text: &str) -> (&str, bool) {
    let mut end = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], end < text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn cold_adapter_retains_completed_provenance_without_source_declaration() {
        let temp = tempfile::tempdir().unwrap();
        let root = tokio::fs::canonicalize(temp.path()).await.unwrap();
        let store = AppStore::open(&root).await.unwrap();
        let created = store
            .create("completed", "Completed monitor")
            .await
            .unwrap();
        store.publish("completed", &created.revision).await.unwrap();
        let secret = root.join("training-data.txt");
        tokio::fs::write(&secret, "local training records")
            .await
            .unwrap();
        let file = FileRecord {
            path: secret.to_string_lossy().into_owned(),
            zone: "training_private".into(),
            attribution: refact_privacy::Attribution::Observed,
        };
        store
            .write_provenance(
                "completed",
                0,
                serde_json::to_value(DurableProvenance {
                    schema_version: 1,
                    incomplete: false,
                    files: vec![file.clone()],
                })
                .unwrap(),
            )
            .await
            .unwrap();
        drop(store);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let service = AutomationService::new(&root).unwrap();
        let adapter = ProcessSourceAdapter::new(gcx, root, service);
        let files = adapter
            .known_records(&created.manifest.app_id.to_string())
            .await
            .unwrap();
        assert_eq!(
            files,
            vec![file],
            "cold context lost persisted process file provenance"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn adapter_rejects_forged_owner_and_joins_shutdown() {
        let temp = tempfile::tempdir().unwrap();
        let root = tokio::fs::canonicalize(temp.path()).await.unwrap();
        let store = AppStore::open(&root).await.unwrap();
        let created = store.create("monitor", "Build monitor").await.unwrap();
        store.publish("monitor", &created.revision).await.unwrap();
        let app_id = created.manifest.app_id.to_string();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let mut owner = crate::chat::types::ChatSession::new("owner".into());
        owner.work_project_root = Some(root.clone());
        gcx.chat_sessions
            .write()
            .await
            .insert("owner".into(), Arc::new(tokio::sync::Mutex::new(owner)));
        let service = AutomationService::new(&root).unwrap();
        let adapter = ProcessSourceAdapter::new(gcx.clone(), root.clone(), service.clone());
        let process = gcx
            .exec_registry
            .register(
                crate::exec::ExecProcessMeta::new(crate::exec::ExecMode::Background, "test".into())
                    .with_chat_id("owner")
                    .with_workspace(root),
                1024,
            )
            .await;
        let def = SourceDefinition {
            source_id: "build".into(),
            channel: "logs".into(),
            permissions: vec![Permission::Process],
            driver: SourceDriver::Process {
                process_id: process.meta.process_id.to_string(),
                chat_id: Some("owner".into()),
            },
        };
        service
            .configure(&app_id, "rev".into(), vec![def.clone()])
            .await
            .unwrap();
        assert!(adapter
            .sync(&app_id, "rev", &[def.clone()], "other")
            .await
            .is_err());
        adapter.sync(&app_id, "rev", &[def], "owner").await.unwrap();
        assert!(adapter.known_records(&app_id).await.is_err());
        gcx.exec_registry
            .append_output(
                &process.meta.process_id,
                crate::exec::ExecOutputStream::Stdout,
                "build ready".into(),
            )
            .await
            .unwrap();
        adapter.pump(Some(&process.meta.process_id), false).await;
        assert!(
            service.snapshot(&app_id).await[0].data.is_none(),
            "passive source leaked output"
        );
        service.set_grant(&app_id, "build", true).await.unwrap();
        service
            .set_demand(&app_id, "build", "test", true)
            .await
            .unwrap();
        adapter.pump(Some(&process.meta.process_id), false).await;
        let event = service.snapshot(&app_id).await.remove(0);
        assert_eq!(event.data.as_ref().unwrap()["text"], "build ready");
        assert_eq!(
            event.data.as_ref().unwrap()["privacy_observation"],
            "unavailable"
        );
        assert!(
            adapter.known_records(&app_id).await.is_err(),
            "unobserved UI bytes became model-public"
        );
        for pause in [true, false] {
            if pause {
                service.pause(&app_id, "build", true).await.unwrap();
            } else {
                service
                    .set_demand(&app_id, "build", "test", false)
                    .await
                    .unwrap();
            }
            let cursor =
                adapter.state.lock().await.targets[&(app_id.clone(), "build".into())].cursor;
            gcx.exec_registry
                .append_output(
                    &process.meta.process_id,
                    crate::exec::ExecOutputStream::Stdout,
                    format!("resumed {pause}"),
                )
                .await
                .unwrap();
            adapter.pump(Some(&process.meta.process_id), false).await;
            assert_eq!(
                adapter.state.lock().await.targets[&(app_id.clone(), "build".into())].cursor,
                cursor,
                "passive adapter advanced past undelivered output"
            );
            if pause {
                service.pause(&app_id, "build", false).await.unwrap();
            } else {
                service
                    .set_demand(&app_id, "build", "test", true)
                    .await
                    .unwrap();
            }
            adapter.pump(Some(&process.meta.process_id), false).await;
            let resumed = service.snapshot(&app_id).await.remove(0);
            assert_eq!(
                resumed.data.as_ref().unwrap()["text"],
                format!("resumed {pause}")
            );
            assert_eq!(resumed.data.as_ref().unwrap()["seq"], cursor);
            adapter.pump(Some(&process.meta.process_id), false).await;
            assert_eq!(
                service.snapshot(&app_id).await[0].seq,
                resumed.seq,
                "resume replayed output"
            );
        }
        service.set_grant(&app_id, "build", false).await.unwrap();
        gcx.exec_registry
            .append_output(
                &process.meta.process_id,
                crate::exec::ExecOutputStream::Stdout,
                "revoked secret".into(),
            )
            .await
            .unwrap();
        adapter.pump(Some(&process.meta.process_id), false).await;
        assert_ne!(
            service.snapshot(&app_id).await[0]
                .data
                .as_ref()
                .and_then(|d| d["text"].as_str()),
            Some("revoked secret")
        );
        let persisted = store.read_provenance("monitor").await.unwrap();
        assert_eq!(persisted.value["incomplete"], true);
        let restarted =
            ProcessSourceAdapter::new(gcx.clone(), adapter.root.clone(), service.clone());
        assert!(
            restarted.known_records(&app_id).await.is_err(),
            "restart erased process privacy taint"
        );
        // Even the same process identity cannot silently adopt a changed policy.
        let retained = service.snapshot(&app_id).await[0].data.clone();
        service
            .set_policy(
                &app_id,
                "build",
                refact_scheduler_core::sources::SourcePolicy {
                    stale_after_ms: 12345,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        service.set_grant(&app_id, "build", true).await.unwrap();
        service
            .set_demand(&app_id, "build", "test", true)
            .await
            .unwrap();
        adapter.pump(Some(&process.meta.process_id), false).await;
        assert_ne!(retained, service.snapshot(&app_id).await[0].data);
        assert!(
            service.snapshot(&app_id).await[0].data.is_none(),
            "changed policy reused trusted process registration"
        );
        // A moved owner cannot adopt the previously authorized process stream.
        adapter
            .authorize(&process.meta.process_id, "owner", &adapter.root)
            .await
            .expect("original pinned owner must be authorized before movement");
        let other = tempfile::tempdir().unwrap();
        {
            let owner = gcx
                .chat_sessions
                .read()
                .await
                .get("owner")
                .cloned()
                .unwrap();
            owner.lock().await.work_project_root = Some(other.path().to_path_buf());
        }
        assert!(adapter
            .authorize(&process.meta.process_id, "owner", &adapter.root)
            .await
            .is_err());
        assert!(adapter.known_records(&app_id).await.is_err());
        let mut task = adapter.start();
        tokio::task::yield_now().await;
        assert!(
            futures::poll!(&mut task).is_pending(),
            "feeder did not remain live before shutdown"
        );
        adapter.shutdown().await;
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(adapter.sync(&app_id, "rev", &[], "owner").await.is_err());
    }
}
