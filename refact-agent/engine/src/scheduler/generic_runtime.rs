use std::{
    path::PathBuf,
    sync::{Arc, Weak},
};

use refact_apps_core::{Permission, SourceDefinition, SourceDriver};
use refact_scheduler_core::{
    automation::{
        AutomationAction, AutomationCatalog, AutomationJournal, AutomationTrigger, CatalogDocument,
        Definition, Lifecycle, ProjectAutomationLease, ReconcileStatus, WakeSummary,
    },
    sources::AutomationService,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::global_context::{GlobalContext, SharedGlobalContext};
use super::{Job, Trigger};

pub const PROJECT_SOURCE_OWNER: &str = "project-automation";

pub struct GenericAutomationRuntime {
    root: PathBuf,
    gcx: Weak<GlobalContext>,
    journal: AutomationJournal,
    lease: Arc<ProjectAutomationLease>,
    pub sources: Arc<AutomationService>,
    gate: Mutex<()>,
    // Human start intent is session-local, bound to the approved canonical digest.
    // Persisted approval alone must not restart a non-resuming background source.
    session_starts: std::sync::Mutex<std::collections::HashMap<String, String>>,
    source_authority: std::sync::Mutex<SourceAuthority>,
    cleanup: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

pub async fn runtime(
    gcx: SharedGlobalContext,
    root: PathBuf,
) -> Result<Arc<GenericAutomationRuntime>, String> {
    Ok(gcx.apps.get(root, gcx.clone()).await?.generic.clone())
}

impl GenericAutomationRuntime {
    pub fn new(
        root: PathBuf,
        gcx: &SharedGlobalContext,
        sources: Arc<AutomationService>,
    ) -> Result<Arc<Self>, String> {
        let lease = ProjectAutomationLease::acquire(&root)?;
        let journal = AutomationJournal::open(&root)?;
        journal.activate_owner(&lease)?;
        let runtime = Arc::new(Self {
            root,
            gcx: Arc::downgrade(gcx),
            journal,
            lease,
            sources,
            gate: Mutex::new(()),
            session_starts: std::sync::Mutex::new(std::collections::HashMap::new()),
            source_authority: Default::default(),
            cleanup: Arc::new(std::sync::Mutex::new(Vec::new())),
        });
        let weak = Arc::downgrade(&runtime);
        runtime
            .sources
            .set_authority_validator(Arc::new(move |app, source| {
                if app != PROJECT_SOURCE_OWNER {
                    return Ok(());
                }
                weak.upgrade()
                    .ok_or("automation runtime unavailable".to_string())?
                    .validate_source_authority(source)
            }));
        Ok(runtime)
    }

    pub async fn shutdown(&self) {
        loop {
            let handles =
                std::mem::take(&mut *self.cleanup.lock().unwrap_or_else(|e| e.into_inner()));
            if handles.is_empty() {
                break;
            }
            for handle in handles {
                let _ = handle.await;
            }
        }
    }

    pub fn list(&self) -> Result<CatalogDocument, String> {
        let document = AutomationCatalog::open(&self.root)?.read()?;
        self.journal.reconcile_catalog(&document)?;
        Ok(document)
    }

    pub fn status(&self, id: &str) -> Result<Option<ReconcileStatus>, String> {
        self.journal.definition_status(id)
    }

    pub fn wake(&self) -> Result<WakeSummary, String> {
        self.journal
            .wake_summary_in_timezone(crate::scheduler::scheduler_timezone())
    }

    pub async fn save(
        self: &Arc<Self>,
        expected_revision: u64,
        definitions: Vec<Definition>,
    ) -> Result<CatalogDocument, String> {
        if definitions.iter().any(|d| {
            !matches!(
                d.owner,
                refact_scheduler_core::automation::DefinitionOwner::Project
            )
        }) {
            return Err("generic catalog accepts only project-owned definitions".into());
        }
        let document = {
            let _gate = self.gate.lock().await;
            AutomationCatalog::open(&self.root)?.compare_and_swap(expected_revision, definitions)?
        };
        self.tick().await?;
        Ok(document)
    }

    /// Trusted confirmation port: callers must already possess human approval for this exact digest.
    pub async fn apply(
        self: &Arc<Self>,
        id: &str,
        digest: &str,
        approved: bool,
    ) -> Result<(), String> {
        {
            let _gate = self.gate.lock().await;
            self.journal.approve_definition(id, digest, approved)?;
            self.record_start(id, digest, approved);
        }
        self.tick().await
    }

    pub async fn apply_exact(
        self: &Arc<Self>,
        id: &str,
        expected_catalog_revision: u64,
        expected_revision: u64,
        digest: &str,
    ) -> Result<(), String> {
        {
            let _gate = self.gate.lock().await;
            let document = self.list()?;
            let definition = document
                .definitions
                .iter()
                .find(|d| d.id == id)
                .ok_or("definition not found")?;
            if document.revision != expected_catalog_revision
                || definition.revision != expected_revision
                || definition.security_digest()? != digest
            {
                return Err("automation approval revision/digest conflict".into());
            }
            validate_execution(definition, None, false)?;
            self.tick_locked().await?;
            self.journal.approve_definition(id, digest, true)?;
            if let AutomationAction::Collect {
                driver, ..
            } = &definition.action
            {
                if let Err(error) = super::source_authorization::authorize_source_bindings(
                    &self.root,
                    &self.sources,
                    PROJECT_SOURCE_OWNER,
                    &Self::source_id(definition)?,
                    driver,
                )
                .await
                {
                    self.journal.approve_definition(id, digest, false)?;
                    return Err(error);
                }
            }
            self.record_start(id, digest, true);
        }
        self.tick().await
    }

    /// Trusted human-confirmation adapter only. Private IPs are exact, never a global private-network grant.
    pub async fn approve_endpoint_exact(
        self: &Arc<Self>,
        id: &str,
        expected_catalog_revision: u64,
        expected_revision: u64,
        digest: &str,
        grant: refact_scheduler_core::sources::EndpointGrant,
    ) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        let document = self.list()?;
        let definition = document
            .definitions
            .iter()
            .find(|d| d.id == id)
            .ok_or("definition not found")?;
        if document.revision != expected_catalog_revision
            || definition.revision != expected_revision
            || definition.security_digest()? != digest
            || !self.allowed(definition)?
        {
            return Err("endpoint approval revision/digest conflict".into());
        }
        let AutomationAction::Collect {
            driver, ..
        } = &definition.action
        else {
            return Err("not a collector".into());
        };
        let url = match driver {
            SourceDriver::HttpPoll {
                url, ..
            }
            | SourceDriver::Sse {
                url,
            }
            | SourceDriver::Websocket {
                url,
            } => url,
            _ => return Err("not a network collector".into()),
        };
        if reqwest::Url::parse(url)
            .map_err(|_| "invalid collector endpoint")?
            .origin()
            .ascii_serialization()
            != grant.origin
        {
            return Err("endpoint approval origin mismatch".into());
        }
        self.tick_locked().await?;
        let source_id = Self::source_id(definition)?;
        super::source_authorization::authorize_source_bindings(
            &self.root,
            &self.sources,
            PROJECT_SOURCE_OWNER,
            &source_id,
            driver,
        )
        .await?;
        self.sources
            .set_endpoint_grant(PROJECT_SOURCE_OWNER, &source_id, Some(grant))
            .await
    }

    /// Trusted exact-human port; the tool confirmation adapter must bind every grant field.
    pub async fn approve_file_exact(
        self: &Arc<Self>,
        id: &str,
        expected_catalog_revision: u64,
        expected_revision: u64,
        digest: &str,
        grant: refact_scheduler_core::sources::FileGrant,
    ) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        let document = self.list()?;
        let definition = document
            .definitions
            .iter()
            .find(|d| d.id == id)
            .ok_or("definition not found")?;
        if document.revision != expected_catalog_revision
            || definition.revision != expected_revision
            || definition.security_digest()? != digest
            || !self.allowed(definition)?
        {
            return Err("file approval revision/digest conflict".into());
        }
        if !matches!(
            &definition.action,
            AutomationAction::Collect {
                driver: SourceDriver::FileTail { .. },
                ..
            }
        ) {
            return Err("not a file collector".into());
        }
        self.tick_locked().await?;
        // The source driver checks exact path and rotation policy, never a directory-wide grant.
        self.sources
            .set_file_grant(
                PROJECT_SOURCE_OWNER,
                &Self::source_id(definition)?,
                Some(grant),
            )
            .await
    }

    pub async fn control(self: &Arc<Self>, id: &str, operation: &str) -> Result<(), String> {
        let document = self.list()?;
        let definition = document
            .definitions
            .iter()
            .find(|d| d.id == id)
            .ok_or("definition not found")?;
        match operation {
            "revoke" | "pause" | "stop" => {
                self.apply(id, &definition.security_digest()?, false).await
            }
            "delete" => self
                .save(
                    document.revision,
                    document
                        .definitions
                        .into_iter()
                        .filter(|d| d.id != id)
                        .collect(),
                )
                .await
                .map(|_| ()),
            "start" | "resume" => Err("start/resume requires exact digest approval".into()),
            _ => Err("unsupported automation control".into()),
        }
    }

    pub fn source_id(definition: &Definition) -> Result<String, String> {
        Ok(format!(
            "a_{}",
            &refact_scheduler_core::automation::digest_json(&json!(definition.id))?[..32]
        ))
    }

    fn allowed(&self, definition: &Definition) -> Result<bool, String> {
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        Ok(definition.enabled
            && !matches!(&definition.lifecycle, Lifecycle::Background { expires_at_ms: Some(at), .. } if *at <= now)
            && self.status(&definition.id)? == Some(ReconcileStatus::Approved))
    }

    fn record_start(&self, id: &str, digest: &str, approved: bool) {
        let mut starts = self
            .session_starts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if approved {
            starts.insert(id.into(), digest.into());
        } else {
            starts.remove(id);
        }
        if approved {
            self.source_authority
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .blocked
                .remove(id);
        }
    }

    fn quarantine_sources(&self) -> Result<(), String> {
        let mut authority = self
            .source_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let ids: Vec<_> = authority
            .compiled
            .values()
            .map(|(id, _)| id.clone())
            .collect();
        authority.blocked.extend(ids);
        self.journal.revoke_compiled_definitions()
    }

    fn validate_source_authority(&self, source: &str) -> Result<(), String> {
        // Read through the bounded, no-follow canonical catalog reader, never a manifest
        // supplied path. No async wait or runtime gate is taken under the source lock.
        let document = match AutomationCatalog::open(&self.root).and_then(|c| c.read()) {
            Ok(document) => document,
            Err(error) => {
                return Err(match self.quarantine_sources() {
                    Ok(()) => error,
                    Err(revoke) => format!("{error}; durable quarantine failed: {revoke}"),
                });
            }
        };
        let mut authority = self
            .source_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (id, digest) = authority
            .compiled
            .get(source)
            .cloned()
            .ok_or("source not compiled")?;
        let valid = !authority.blocked.contains(&id)
            && document.definitions.iter().any(|d| {
                d.id == id
                    && d.security_digest().ok().as_deref() == Some(digest.as_str())
                    && self.allowed(d).unwrap_or(false)
            });
        if !valid {
            authority.blocked.insert(id.clone());
            self.journal.revoke_compiled_definition(&id, &digest)?;
            return Err("canonical source authority changed".into());
        }
        Ok(())
    }

    pub async fn tick(self: &Arc<Self>) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        self.tick_locked().await
    }

    async fn tick_locked(self: &Arc<Self>) -> Result<(), String> {
        let document = match self.list() {
            Ok(document) => document,
            Err(error) => {
                let quarantine = self.quarantine_sources();
                // unconfigure cancels first and retains a passive hold if tombstoning fails.
                self.sources
                    .unconfigure(PROJECT_SOURCE_OWNER)
                    .await
                    .map_err(|cleanup| format!("{error}; source quarantine failed: {cleanup}"))?;
                quarantine
                    .map_err(|revoke| format!("{error}; durable quarantine failed: {revoke}"))?;
                return Err(error);
            }
        };
        let mut definitions = Vec::new();
        let mut approved = Vec::new();
        for definition in &document.definitions {
            if let AutomationAction::Collect {
                driver, ..
            } = &definition.action
            {
                let channel = definition
                    .deliveries
                    .first()
                    .map(|d| d.channel.clone())
                    .unwrap_or_else(|| "events".into());
                let permissions = match driver {
                    SourceDriver::Internal {
                        topic,
                    } => vec![Permission::Internal {
                        topic: topic.clone(),
                    }],
                    SourceDriver::HttpPoll {
                        url, ..
                    }
                    | SourceDriver::Sse {
                        url,
                    }
                    | SourceDriver::Websocket {
                        url,
                    } => vec![Permission::Network {
                        origin: reqwest::Url::parse(url)
                            .map_err(|_| "invalid collector endpoint")?
                            .origin()
                            .ascii_serialization(),
                    }],
                    _ => Vec::new(),
                };
                let source_id = Self::source_id(definition)?;
                self.source_authority
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .compiled
                    .insert(
                        source_id.clone(),
                        (definition.id.clone(), definition.security_digest()?),
                    );
                definitions.push(SourceDefinition {
                    source_id: source_id.clone(),
                    channel,
                    driver: driver.clone(),
                    permissions,
                });
                approved.push((definition, source_id));
            }
        }
        self.sources
            .configure(
                PROJECT_SOURCE_OWNER,
                document.revision.to_string(),
                definitions,
            )
            .await?;
        for (definition, source_id) in approved {
            if let AutomationAction::Collect {
                policy, ..
            } = &definition.action
            {
                if policy.owner_definition_digest.is_some() {
                    return Err("owner definition digest is host-only".into());
                }
                let mut compiled_policy = policy.clone();
                compiled_policy.owner_definition_digest = Some(definition.security_digest()?);
                self.sources
                    .set_policy(PROJECT_SOURCE_OWNER, &source_id, compiled_policy)
                    .await?;
            }
            let allowed = self.allowed(definition)?
                && !self
                    .source_authority
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .blocked
                    .contains(&definition.id);
            self.sources
                .set_grant(PROJECT_SOURCE_OWNER, &source_id, allowed)
                .await?;
            let (background, resume_after_restart, expires_at_ms) = match definition.lifecycle {
                Lifecycle::Background {
                    resume_after_restart,
                    expires_at_ms,
                } => {
                    let started = self
                        .session_starts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&definition.id)
                        .cloned();
                    (
                        allowed
                            && matches!(definition.trigger, AutomationTrigger::Continuous)
                            && (resume_after_restart
                                || started.as_deref()
                                    == Some(definition.security_digest()?.as_str())),
                        resume_after_restart,
                        expires_at_ms,
                    )
                }
                Lifecycle::OnDemand => (false, false, None),
            };
            let digest = self
                .sources
                .source_config_digest(PROJECT_SOURCE_OWNER, &source_id)
                .await?;
            self.sources
                .set_source_control(
                    PROJECT_SOURCE_OWNER,
                    &source_id,
                    &digest,
                    refact_scheduler_core::automation::SourceControl {
                        background,
                        resume_after_restart: background && resume_after_restart,
                        expires_at_ms,
                        ..Default::default()
                    },
                )
                .await?;
        }
        Ok(())
    }

    pub async fn run_exact(
        self: &Arc<Self>,
        id: &str,
        expected_catalog_revision: u64,
        expected_revision: u64,
    ) -> Result<Value, String> {
        let definition = {
            let _gate = self.gate.lock().await;
            let document = self.list()?;
            let definition = document
                .definitions
                .into_iter()
                .find(|d| d.id == id)
                .ok_or("definition not found")?;
            if document.revision != expected_catalog_revision
                || definition.revision != expected_revision
                || !self.allowed(&definition)?
            {
                return Err("automation run revision/approval conflict".into());
            }
            definition
        };
        self.execute(
            &definition,
            &format!("manual:{}", uuid::Uuid::new_v4()),
            None,
        )
        .await
    }

    pub async fn revoke_exact(
        self: &Arc<Self>,
        id: &str,
        expected_catalog_revision: u64,
        expected_revision: u64,
    ) -> Result<(), String> {
        {
            let _gate = self.gate.lock().await;
            let document = self.list()?;
            let definition = document
                .definitions
                .iter()
                .find(|d| d.id == id)
                .ok_or("definition not found")?;
            if document.revision != expected_catalog_revision
                || definition.revision != expected_revision
            {
                return Err("automation revoke revision conflict".into());
            }
            self.journal
                .approve_definition(id, &definition.security_digest()?, false)?;
            self.record_start(id, &definition.security_digest()?, false);
        }
        self.tick().await
    }

    pub async fn run(self: &Arc<Self>, id: &str) -> Result<Value, String> {
        let definition = self
            .list()?
            .definitions
            .into_iter()
            .find(|d| d.id == id)
            .ok_or("definition not found")?;
        if !self.allowed(&definition)? {
            return Err("automation requires exact digest approval".into());
        }
        let gcx = self.gcx.upgrade().ok_or("automation runtime unavailable")?;
        if !super::runner::runner_enabled(&gcx.scheduler_config) {
            return Err("scheduler disabled".into());
        }
        self.execute(
            &definition,
            &format!("manual:{}", uuid::Uuid::new_v4()),
            None,
        )
        .await
    }

    pub async fn dispatch_due(self: &Arc<Self>) -> Result<(), String> {
        self.tick().await?;
        self.drain_deliveries().await?;
        let now = now_ms();
        for definition in self.list()?.definitions {
            if !self.allowed(&definition)? {
                continue;
            }
            let AutomationTrigger::Schedule {
                schedule,
            } = &definition.trigger
            else {
                continue;
            };
            let Some(state) = self.journal.definition_schedule_state(&definition.id)? else {
                continue;
            };
            let mut job = legacy_job(
                &definition,
                super::Action::AgentTurn {
                    prompt: String::new(),
                    target: super::AgentTarget::Isolated,
                    mode: None,
                    model: None,
                    tools: None,
                },
                schedule.clone(),
                &definition.id,
            )?;
            job.created_at_ms = state.created_at_ms;
            job.last_fired_at_ms = state.last_fired_at_ms;
            let gcx = self.gcx.upgrade().ok_or("automation runtime unavailable")?;
            let grace = super::schedule::MissedRunGraceConfig::from(&gcx.scheduler_config);
            let from = job.last_fired_at_ms.unwrap_or(job.created_at_ms);
            if let Some(missed) = super::schedule::recurring_missed_grace_state(
                &job,
                from,
                now,
                super::scheduler_timezone(),
                grace,
            ) {
                if missed.due_ms.is_some() && !missed.should_fire {
                    if let Some(anchor) = missed.advance_last_fired_at_ms {
                        self.journal.advance_definition_schedule(
                            &definition.id,
                            &definition.security_digest()?,
                            anchor,
                        )?;
                    }
                    continue;
                }
            }
            let due = super::runner::scheduled_fire_at_ms(
                &job,
                now,
                &super::jitter::JitterConfig::default(),
                grace,
            );
            if let Some(at) = due.filter(|at| *at <= now) {
                match self
                    .execute(&definition, &format!("schedule:{at}"), None)
                    .await
                {
                    Ok(_) => {
                        self.journal.advance_definition_schedule(
                            &definition.id,
                            &definition.security_digest()?,
                            at,
                        )?;
                    }
                    Err(error) => {
                        // Capacity is transient; a terminal attempt must not hot-loop forever.
                        if error != "scheduler at capacity" {
                            self.journal.advance_definition_schedule(
                                &definition.id,
                                &definition.security_digest()?,
                                at,
                            )?;
                        }
                        tracing::warn!(automation_id=%definition.id,"automation occurrence failed: {error}");
                    }
                }
            }
        }
        Ok(())
    }

    async fn execute(
        self: &Arc<Self>,
        definition: &Definition,
        occurrence: &str,
        payload: Option<&Value>,
    ) -> Result<Value, String> {
        let gcx = self.gcx.upgrade().ok_or("automation runtime unavailable")?;
        if !super::runner::runner_enabled(&gcx.scheduler_config) {
            return Err("scheduler disabled".into());
        }
        let _permit = self
            .lease
            .capacity("execution", gcx.scheduler_config.max_concurrent_runs.max(1))
            .try_acquire_owned()
            .map_err(|_| "scheduler at capacity")?;
        validate_execution(definition, payload, true)?;
        let Some(claim) =
            self.journal
                .claim_definition_run(&definition.id, occurrence, self.lease.epoch())?
        else {
            return Ok(json!({"admitted":false}));
        };
        let mut effect = Effect {
            journal: &self.journal,
            claim,
            finished: false,
            started: false,
            ambiguous: matches!(definition.action, AutomationAction::Legacy { .. }),
        };
        if definition.security_digest()? != effect.claim.config_digest {
            return Err("automation snapshot changed before admission".into());
        }
        if !self.journal.mark_effect_started(&effect.claim)? {
            return Err("automation authorization changed".into());
        }
        effect.started = true;
        let result = match &definition.action {
            AutomationAction::Legacy {
                action,
            } => {
                let job = legacy_job(definition, action.clone(), Trigger::Manual, &definition.id)?;
                super::CronRunner::execute_definition_legacy(gcx, &job, occurrence).await
            }
            AutomationAction::Collect {
                ..
            } => {
                let id = Self::source_id(definition)?;
                let consumer = format!("run:{}", effect.claim.run_id);
                let mut demand = FiniteDemand {
                    sources: self.sources.clone(),
                    source: id.clone(),
                    consumer: Some(consumer.clone()),
                    cleanup: self.cleanup.clone(),
                };
                let result = match self
                    .sources
                    .set_demand(PROJECT_SOURCE_OWNER, &id, &consumer, true)
                    .await
                {
                    Ok(()) => self.sources.refresh(PROJECT_SOURCE_OWNER, &id).await,
                    Err(error) => Err(error),
                };
                if self
                    .sources
                    .set_demand(PROJECT_SOURCE_OWNER, &id, &consumer, false)
                    .await
                    .is_ok()
                {
                    demand.consumer = None;
                }
                result.and_then(|event| serde_json::to_value(event).map_err(|e| e.to_string()))
            }
            AutomationAction::Transform {
                input_channel,
                output_channel,
                select_fields,
                max_output_bytes,
            } => {
                let payload = payload.ok_or("transform requires channel event payload")?;
                if !matches!(&definition.trigger, AutomationTrigger::ChannelEvent { channel, .. } if channel == input_channel)
                {
                    return Err("transform input mismatch".into());
                }
                if !definition
                    .deliveries
                    .iter()
                    .any(|d| d.channel == *output_channel && d.credential_ref.is_none())
                {
                    return Err("transform output channel is not approved delivery".into());
                }
                let output = select(payload, select_fields, *max_output_bytes)?;
                Ok(output)
            }
        };
        let status = if result.is_ok() {
            refact_scheduler_core::automation::RunStatus::Succeeded
        } else if matches!(definition.action, AutomationAction::Legacy { .. }) {
            refact_scheduler_core::automation::RunStatus::Unknown
        } else {
            refact_scheduler_core::automation::RunStatus::Failed
        };
        let delivery = result.as_ref().ok().map(|value| json!({"definition_id":definition.id,"digest":effect.claim.config_digest,"run_id":effect.claim.run_id,"occurrence":occurrence,"value":value,"deliveries":definition.deliveries}));
        self.journal.complete_run_and_enqueue_delivery(
            &effect.claim,
            status,
            result.as_ref().ok(),
            result.as_ref().err().map(String::as_str),
            delivery
                .as_ref()
                .filter(|_| {
                    !definition.deliveries.is_empty()
                        && !matches!(definition.action, AutomationAction::Collect { .. })
                })
                .map(|value| (value, "automation_channels")),
        )?;
        effect.finished = true;
        result
    }

    pub async fn drain_deliveries(self: &Arc<Self>) -> Result<(), String> {
        for delivery in self.journal.pending_deliveries_for_targets(
            &["automation_source_channels".into()],
            64,
            now_ms(),
        )? {
            let result = async {
                let envelope = &delivery.data;
                let app = envelope["app_id"]
                    .as_str()
                    .ok_or("invalid source envelope")?;
                let source = envelope["source_id"]
                    .as_str()
                    .ok_or("invalid source envelope")?;
                envelope["channel"]
                    .as_str()
                    .ok_or("invalid source envelope")?;
                if app != PROJECT_SOURCE_OWNER {
                    return Err("source reaction owner mismatch".to_string());
                }
                let digest = self.sources.source_config_digest(app, source).await?;
                if envelope["source_config_digest"].as_str() != Some(digest.as_str()) {
                    return Err("source reaction digest changed".into());
                }
                let definition = self
                    .list()?
                    .definitions
                    .into_iter()
                    .find(|d| Self::source_id(d).ok().as_deref() == Some(source))
                    .ok_or("source definition removed")?;
                if !self.allowed(&definition)? {
                    return Err("source reaction authorization changed".into());
                }
                // Source ingest is the single collector data path, including finite
                // runs. Completion records must not emit a second SourceEvent wrapper.
                for target in &definition.deliveries {
                    if target.credential_ref.is_some() {
                        return Err("credential-bearing channel delivery unsupported".into());
                    }
                    let event_id = format!("source:{}:{}", delivery.event_seq, target.id);
                    crate::apps::automation_channels::publish(
                        &self.sources,
                        &target.channel,
                        envelope["data"].clone(),
                        &event_id,
                        &definition.id,
                        Some(source),
                        &digest,
                        delivery.event_seq,
                    )
                    .await?;
                    self.emit_event_id(&target.channel, envelope["data"].clone(), &event_id, 0)
                        .await?;
                }
                Ok::<(), String>(())
            }
            .await;
            match result {
                Ok(()) => {
                    self.journal
                        .acknowledge_delivery(delivery.event_seq, &delivery.target)?;
                }
                Err(error) => {
                    self.journal.retry_delivery(
                        delivery.event_seq,
                        &delivery.target,
                        now_ms().saturating_add(60_000),
                        &error,
                    )?;
                }
            }
        }
        for delivery in self.journal.pending_deliveries_for_targets(
            &["automation_channels".into()],
            64,
            now_ms(),
        )? {
            let result = async {
                let id = delivery.data["definition_id"]
                    .as_str()
                    .ok_or("invalid delivery definition")?;
                let definition = self
                    .list()?
                    .definitions
                    .into_iter()
                    .find(|d| d.id == id)
                    .ok_or("delivery definition removed")?;
                if !self.allowed(&definition)?
                    || delivery.data["digest"].as_str()
                        != Some(definition.security_digest()?.as_str())
                {
                    return Err("delivery approval changed".to_string());
                }
                let value = delivery.data["value"].clone();
                let occurrence = delivery.data["occurrence"].as_str().unwrap_or("");
                let depth = occurrence
                    .strip_prefix("event:")
                    .and_then(|s| s.split(':').next())
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                if depth >= 8 {
                    return Err("reaction depth limit".into());
                }
                for target in &definition.deliveries {
                    // Stable outbox identity fences each downstream execution even after a crash.
                    let event_id = format!("outbox:{}:{}", delivery.event_seq, target.id);
                    crate::apps::automation_channels::publish(
                        &self.sources,
                        &target.channel,
                        value.clone(),
                        &event_id,
                        &definition.id,
                        None,
                        &definition.security_digest()?,
                        delivery.event_seq,
                    )
                    .await?;
                    self.emit_event_id(&target.channel, value.clone(), &event_id, depth + 1)
                        .await?;
                }
                Ok::<(), String>(())
            }
            .await;
            match result {
                Ok(()) => {
                    self.journal
                        .acknowledge_delivery(delivery.event_seq, &delivery.target)?;
                }
                Err(error) => {
                    self.journal.retry_delivery(
                        delivery.event_seq,
                        &delivery.target,
                        now_ms().saturating_add(60_000),
                        &error,
                    )?;
                }
            }
        }
        Ok(())
    }

    pub async fn emit_event(
        self: &Arc<Self>,
        channel: &str,
        payload: Value,
    ) -> Result<usize, String> {
        let event_id = refact_scheduler_core::automation::digest_json(&payload)?;
        self.emit_event_id(channel, payload, &event_id, 0).await
    }

    pub async fn emit_event_id(
        self: &Arc<Self>,
        channel: &str,
        payload: Value,
        event_id: &str,
        depth: usize,
    ) -> Result<usize, String> {
        if depth >= 8 {
            return Err("reaction depth limit".into());
        }
        let gcx = self.gcx.upgrade().ok_or("automation runtime unavailable")?;
        if !super::runner::runner_enabled(&gcx.scheduler_config) {
            return Err("scheduler disabled".into());
        }
        let mut count = 0;
        for definition in self.list()?.definitions {
            let AutomationTrigger::ChannelEvent {
                channel: input,
                filter,
            } = &definition.trigger
            else {
                continue;
            };
            if input != channel
                || !matches_filter(filter, &payload)
                || !self.allowed(&definition)?
            {
                continue;
            }
            if serde_json::to_vec(&payload)
                .map_err(|e| e.to_string())?
                .len()
                > definition.policy.max_event_bytes
            {
                return Err("event bounds".into());
            }
            let stable_id =
                refact_scheduler_core::automation::digest_json(&json!([channel, event_id]))?;
            self.execute(
                &definition,
                &format!("event:{depth}:{stable_id}"),
                Some(&payload),
            )
            .await?;
            count += 1;
        }
        Ok(count)
    }
}

#[derive(Default)]
struct SourceAuthority {
    compiled: std::collections::HashMap<String, (String, String)>,
    blocked: std::collections::HashSet<String>,
}

struct FiniteDemand {
    sources: Arc<AutomationService>,
    source: String,
    consumer: Option<String>,
    cleanup: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}
impl Drop for FiniteDemand {
    fn drop(&mut self) {
        if let (Some(consumer), Ok(handle)) =
            (self.consumer.take(), tokio::runtime::Handle::try_current())
        {
            let sources = self.sources.clone();
            let source = self.source.clone();
            let mut cleanup = self.cleanup.lock().unwrap_or_else(|e| e.into_inner());
            cleanup.retain(|task| !task.is_finished());
            cleanup.push(handle.spawn(async move {
                if let Err(error) = sources
                    .set_demand(PROJECT_SOURCE_OWNER, &source, &consumer, false)
                    .await
                {
                    tracing::warn!("cancelled automation demand cleanup failed: {error}");
                }
            }));
        }
    }
}

struct Effect<'a> {
    journal: &'a AutomationJournal,
    claim: refact_scheduler_core::automation::RunClaim,
    finished: bool,
    started: bool,
    ambiguous: bool,
}
impl Drop for Effect<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.journal.finish_run(
                &self.claim,
                if self.started && self.ambiguous {
                    refact_scheduler_core::automation::RunStatus::Unknown
                } else {
                    refact_scheduler_core::automation::RunStatus::Failed
                },
                None,
                Some("effect interrupted"),
            );
        }
    }
}

fn validate_execution(
    definition: &Definition,
    payload: Option<&Value>,
    executing: bool,
) -> Result<(), String> {
    definition.validate()?;
    if !matches!(
        definition.owner,
        refact_scheduler_core::automation::DefinitionOwner::Project
    ) {
        return Err("generic execution requires project owner".into());
    }
    if definition
        .deliveries
        .iter()
        .any(|d| d.credential_ref.is_some())
    {
        return Err("credential-bearing channel deliveries are not supported".into());
    }
    if let AutomationAction::Transform {
        input_channel,
        output_channel,
        select_fields,
        max_output_bytes,
    } = &definition.action
    {
        if !matches!(&definition.trigger, AutomationTrigger::ChannelEvent { channel, .. } if channel == input_channel)
        {
            return Err("transform input mismatch".into());
        }
        if !definition
            .deliveries
            .iter()
            .any(|d| d.channel == *output_channel)
        {
            return Err("transform output channel is not approved delivery".into());
        }
        if executing {
            select(
                payload.ok_or("transform requires channel event payload")?,
                select_fields,
                *max_output_bytes,
            )?;
        }
    }
    if let AutomationAction::Collect {
        driver,
        policy,
    } = &definition.action
    {
        if policy.owner_definition_digest.is_some() {
            return Err("owner definition digest is host-only".into());
        }
        policy.validate()?;
        if policy.credential_bindings.iter().any(|b| {
            b.expected_identity_revision
                .as_ref()
                .is_none_or(|r| r.is_empty())
        }) {
            return Err("credential binding requires expected identity revision".into());
        }
        if executing
            && !matches!(
                driver,
                SourceDriver::Interval { .. } | SourceDriver::HttpPoll { .. }
            )
        {
            return Err("finite collector execution supports interval and HTTP polling only; streaming sources use continuous lifecycle".into());
        }
    }
    if let AutomationAction::Legacy {
        action,
    } = &definition.action
    {
        let job = legacy_job(definition, action.clone(), Trigger::Manual, &definition.id)?;
        super::CronRunner::validate_definition_legacy(&job)?;
    }
    Ok(())
}

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}
fn legacy_job(
    definition: &Definition,
    action: super::Action,
    trigger: Trigger,
    id: &str,
) -> Result<Job, String> {
    let delivery = if matches!(action, super::Action::AgentTurn { .. }) {
        json!({"kind":"chat"})
    } else {
        json!({"kind":"none"})
    };
    serde_json::from_value(json!({"id":id,"description":definition.description,"enabled":true,"durable":false,"created_at_ms":now_ms(),"recurring": !matches!(trigger, Trigger::Once { .. }),"trigger":trigger,"action":action,"delivery":delivery})).map_err(|e| e.to_string())
}
fn matches_filter(filter: &Value, payload: &Value) -> bool {
    filter.is_null()
        || filter
            .as_object()
            .is_some_and(|fields| fields.iter().all(|(k, v)| payload.get(k) == Some(v)))
}
fn select(payload: &Value, fields: &[String], max: usize) -> Result<Value, String> {
    if fields.len() > 128 || max > 262144 {
        return Err("transform bounds".into());
    }
    let mut output = serde_json::Map::new();
    for field in fields {
        if let Some(value) = payload.get(field) {
            output.insert(field.clone(), value.clone());
        }
    }
    let output = Value::Object(output);
    if serde_json::to_vec(&output)
        .map_err(|e| e.to_string())?
        .len()
        > max
    {
        return Err("transform output bounds".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn continuous_definition(resume: bool) -> Definition {
        serde_json::from_value(json!({"schema_version":1,"id":"continuous_test","revision":1,"owner":{"kind":"project"},"description":"test","enabled":true,"trigger":{"kind":"continuous"},"action":{"kind":"collect","driver":{"type":"interval","every_ms":1000}},"deliveries":[],"lifecycle":{"kind":"background","resume_after_restart":resume,"expires_at_ms":null},"policy":{"idempotent":true,"max_pending_events":16,"max_event_bytes":1024}})).unwrap()
    }

    async fn next_data(events: &mut refact_scheduler_core::sources::SourceSubscription) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if matches!(
                    events.recv().await.unwrap().status,
                    refact_scheduler_core::sources::SourceStatus::Data
                ) {
                    break;
                }
            }
        })
        .await
        .expect("real interval source produced no output");
    }

    #[tokio::test]
    async fn human_credential_pipeline_uses_private_store_and_fences_identity_change() {
        use refact_scheduler_core::sources::{
            CredentialBinding, CredentialContext, SourceCredentialResolver, SourcePolicy,
        };
        use crate::scheduler::credentials::{AutomationCredentialStore, CredentialIdentity};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let store = AutomationCredentialStore::open(temp.path(), None).unwrap();
        let identity = CredentialIdentity {
            id: "pipeline_fixture".into(),
            origin: origin.clone(),
            header: "authorization".into(),
            tenant: "fixture_tenant".into(),
            principal: "fixture_principal".into(),
            scope: vec!["read".into()],
        };
        let metadata = store
            .set(identity.clone(), "fixture_first".into(), None)
            .unwrap();
        let binding = CredentialBinding {
            reference: identity.id.clone(),
            header: identity.header.clone(),
            scheme: Some("Bearer".into()),
            expected_identity_revision: Some(metadata.identity_revision.clone()),
        };
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        let mut definition = continuous_definition(false);
        definition.lifecycle = Lifecycle::OnDemand;
        definition.action = AutomationAction::Collect {
            driver: SourceDriver::HttpPoll {
                url: format!("{origin}/data"),
                every_ms: 1000,
            },
            policy: SourcePolicy {
                credential_bindings: vec![binding.clone()],
                ..Default::default()
            },
        };
        let mut changed = definition.clone();
        if let AutomationAction::Collect {
            policy, ..
        } = &mut changed.action
        {
            policy.timeout_ms += 1;
        }
        assert_ne!(
            definition.security_digest().unwrap(),
            changed.security_digest().unwrap()
        );
        let document = host
            .generic
            .save(0, vec![definition.clone()])
            .await
            .unwrap();
        let source = GenericAutomationRuntime::source_id(&definition).unwrap();
        let source_digest = host
            .automation
            .source_config_digest(PROJECT_SOURCE_OWNER, &source)
            .await
            .unwrap();
        let context = CredentialContext {
            app_id: PROJECT_SOURCE_OWNER,
            source_id: &source,
            config_digest: &source_digest,
            origin: &origin,
        };
        assert!(
            store.resolve(&context, &binding).is_err(),
            "desired policy must not authorize a reference"
        );
        assert!(host
            .generic
            .apply_exact(
                &definition.id,
                document.revision + 1,
                definition.revision,
                &definition.security_digest().unwrap()
            )
            .await
            .is_err());
        assert!(store.resolve(&context, &binding).is_err());
        host.generic
            .apply_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        host.generic
            .approve_endpoint_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &definition.security_digest().unwrap(),
                refact_scheduler_core::sources::EndpointGrant {
                    origin: origin.clone(),
                    allowed_private_ips: vec![address.ip()],
                },
            )
            .await
            .unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                observed
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request[..n]).to_string());
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}").await;
            }
        });
        host.generic
            .run_exact(&definition.id, document.revision, definition.revision)
            .await
            .unwrap();
        let rotated = store
            .rotate(
                &identity.id,
                &metadata.secret_revision,
                "fixture_rotated".into(),
            )
            .unwrap();
        assert_eq!(metadata.identity_revision, rotated.identity_revision);
        host.generic
            .run_exact(&definition.id, document.revision, definition.revision)
            .await
            .unwrap();
        server.abort();
        assert!(requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("Bearer fixture_first")));
        assert!(requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("Bearer fixture_rotated")));
        let mut changed_identity = identity;
        changed_identity.scope = vec!["other".into()];
        let changed = store
            .set(
                changed_identity,
                "fixture_changed".into(),
                Some(&rotated.secret_revision),
            )
            .unwrap();
        assert_ne!(metadata.identity_revision, changed.identity_revision);
        assert!(store.resolve(&context, &binding).is_err());
        host.generic.tick().await.unwrap();
        assert!(
            store.resolve(&context, &binding).is_err(),
            "tick must never reauthorize changed identity"
        );
        assert!(host
            .automation
            .publish(PROJECT_SOURCE_OWNER, &source, json!({"forbidden":true}))
            .await
            .is_err());
        gcx.apps.shutdown().await;
    }

    #[tokio::test]
    async fn non_resuming_source_starts_this_session_but_not_on_host_rebuild() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        let definition = continuous_definition(false);
        let document = host
            .generic
            .save(0, vec![definition.clone()])
            .await
            .unwrap();
        let mut events = host.automation.subscribe();
        host.generic
            .apply_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        next_data(&mut events).await;
        gcx.apps.shutdown().await;
        drop(host);
        // Real App host startup, with no document/UI fetch, must not recreate demand.
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        host.generic.tick().await.unwrap();
        let id = GenericAutomationRuntime::source_id(&definition).unwrap();
        assert!(host
            .automation
            .publish(PROJECT_SOURCE_OWNER, &id, json!("unexpected restart"))
            .await
            .is_err());
        let state = host
            .generic
            .journal
            .source_state(PROJECT_SOURCE_OWNER, &id)
            .unwrap()
            .unwrap();
        assert!(!state.control.background);
        // A finite explicit manual occurrence still works without background restart intent.
        host.generic
            .run_exact(&definition.id, document.revision, definition.revision)
            .await
            .unwrap();
        assert!(
            !host
                .generic
                .journal
                .source_state(PROJECT_SOURCE_OWNER, &id)
                .unwrap()
                .unwrap()
                .control
                .background
        );
        // Approval persisted, but a fresh human start can run it in this session again.
        let mut events = host.automation.subscribe();
        host.generic
            .apply_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        next_data(&mut events).await;
        gcx.apps.shutdown().await;
    }

    #[tokio::test]
    async fn invalid_catalog_fences_an_already_pending_http_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            accepted_tx.send(()).unwrap();
            release_rx.await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 18\r\nConnection: close\r\n\r\n{\"forbidden\":true}").await.unwrap();
        });
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        let mut definition = continuous_definition(true);
        definition.action = AutomationAction::Collect {
            driver: SourceDriver::HttpPoll {
                url: format!("http://{address}/data"),
                every_ms: 1000,
            },
            policy: Default::default(),
        };
        let document = host
            .generic
            .save(0, vec![definition.clone()])
            .await
            .unwrap();
        let digest = definition.security_digest().unwrap();
        host.generic
            .apply_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &digest,
            )
            .await
            .unwrap();
        host.generic
            .approve_endpoint_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &digest,
                refact_scheduler_core::sources::EndpointGrant {
                    origin: format!("http://{address}"),
                    allowed_private_ips: vec![address.ip()],
                },
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), accepted_rx)
            .await
            .unwrap()
            .unwrap();
        let mut events = host.automation.subscribe();
        std::fs::write(temp.path().join(".refact/automations.json"), b"invalid").unwrap();
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let event = events.recv().await.unwrap();
                assert!(
                    !matches!(
                        event.status,
                        refact_scheduler_core::sources::SourceStatus::Data
                    ),
                    "in-flight response crossed canonical fence"
                );
                if event.health["error_class"] == "canonical_authority_denied" {
                    break;
                }
            }
        })
        .await
        .unwrap();
        server.await.unwrap();
        gcx.apps.shutdown().await;
    }

    #[tokio::test]
    async fn invalid_catalog_blocks_source_publish_before_tick_and_recovery_requires_approval() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        let definition = continuous_definition(true);
        let document = host
            .generic
            .save(0, vec![definition.clone()])
            .await
            .unwrap();
        host.generic
            .apply_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        let id = GenericAutomationRuntime::source_id(&definition).unwrap();
        let path = temp.path().join(".refact/automations.json");
        let valid = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"{invalid").unwrap();
        assert!(
            host.automation
                .publish(PROJECT_SOURCE_OWNER, &id, json!({"forbidden":true}))
                .await
                .is_err(),
            "forwarding fence must not wait for tick"
        );
        assert!(host.generic.tick().await.is_err());
        std::fs::write(&path, valid).unwrap();
        host.generic.tick().await.unwrap();
        assert!(
            host.automation
                .publish(PROJECT_SOURCE_OWNER, &id, json!({"forbidden":true}))
                .await
                .is_err(),
            "repair must not reset quarantine"
        );
        gcx.apps.shutdown().await;
        drop(host);
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        assert_ne!(
            host.generic.status(&definition.id).unwrap(),
            Some(ReconcileStatus::Approved)
        );
        assert!(
            host.automation
                .publish(PROJECT_SOURCE_OWNER, &id, json!({"forbidden":true}))
                .await
                .is_err(),
            "host rebuild must not reset quarantine"
        );
        let mut events = host.automation.subscribe();
        host.generic
            .apply_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        next_data(&mut events).await;
        gcx.apps.shutdown().await;
    }

    fn transform_definition() -> Definition {
        serde_json::from_value(json!({"schema_version":1,"id":"transform_test","revision":1,"owner":{"kind":"project"},"description":"test","enabled":true,"trigger":{"kind":"channel_event","channel":"input","filter":null},"action":{"kind":"transform","input_channel":"input","output_channel":"output","select_fields":["public"],"max_output_bytes":1024},"deliveries":[{"id":"output","channel":"output","credential_ref":null}],"lifecycle":{"kind":"on_demand"},"policy":{"idempotent":true,"max_pending_events":16,"max_event_bytes":1024}})).unwrap()
    }

    #[tokio::test]
    async fn exact_approval_and_stale_action_snapshot_are_fenced() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let sources = AutomationService::new(temp.path()).unwrap();
        let runtime =
            GenericAutomationRuntime::new(temp.path().into(), &gcx, sources.clone()).unwrap();
        let first = transform_definition();
        let document = runtime.save(0, vec![first.clone()]).await.unwrap();
        assert!(runtime
            .apply_exact(
                &first.id,
                document.revision + 1,
                first.revision,
                &first.security_digest().unwrap()
            )
            .await
            .is_err());
        runtime
            .apply_exact(
                &first.id,
                document.revision,
                first.revision,
                &first.security_digest().unwrap(),
            )
            .await
            .unwrap();
        let mut second = first.clone();
        second.revision += 1;
        if let AutomationAction::Transform {
            select_fields, ..
        } = &mut second.action
        {
            *select_fields = vec!["other".into()];
        }
        let document = runtime
            .save(document.revision, vec![second.clone()])
            .await
            .unwrap();
        runtime
            .apply_exact(
                &second.id,
                document.revision,
                second.revision,
                &second.security_digest().unwrap(),
            )
            .await
            .unwrap();
        assert!(runtime
            .execute(
                &first,
                "event:0:stale",
                Some(&json!({"public":1,"other":2}))
            )
            .await
            .unwrap_err()
            .contains("snapshot changed"));
        runtime
            .execute(
                &second,
                "event:0:valid",
                Some(&json!({"public":1,"other":2})),
            )
            .await
            .unwrap();
        assert_eq!(
            runtime
                .journal
                .pending_deliveries_for_targets(&["automation_channels".into()], 64, now_ms())
                .unwrap()
                .len(),
            1
        );
        runtime.drain_deliveries().await.unwrap();
        assert!(runtime
            .journal
            .pending_deliveries_for_targets(&["automation_channels".into()], 64, now_ms())
            .unwrap()
            .is_empty());
        runtime
            .execute(&second, "event:0:valid", Some(&json!({"other":3})))
            .await
            .unwrap();
        assert!(
            runtime
                .journal
                .pending_deliveries_for_targets(&["automation_channels".into()], 64, now_ms())
                .unwrap()
                .is_empty(),
            "duplicate occurrence produced another delivery"
        );
        sources.shutdown().await;
    }

    #[tokio::test]
    async fn cancelled_finite_demand_cleanup_is_owned_and_joined() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let sources = AutomationService::new(temp.path()).unwrap();
        let runtime =
            GenericAutomationRuntime::new(temp.path().into(), &gcx, sources.clone()).unwrap();
        let mut definition = continuous_definition(false);
        definition.lifecycle = Lifecycle::OnDemand;
        definition.action = AutomationAction::Collect {
            driver: SourceDriver::Manual,
            policy: Default::default(),
        };
        let document = runtime.save(0, vec![definition.clone()]).await.unwrap();
        runtime
            .apply_exact(
                &definition.id,
                document.revision,
                definition.revision,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        let source = GenericAutomationRuntime::source_id(&definition).unwrap();
        sources
            .set_demand(PROJECT_SOURCE_OWNER, &source, "run:test", true)
            .await
            .unwrap();
        // Prove the compiled authority and pending demand are genuinely usable.
        sources
            .publish(PROJECT_SOURCE_OWNER, &source, json!({"before_drop":true}))
            .await
            .unwrap();
        drop(FiniteDemand {
            sources: sources.clone(),
            source: source.clone(),
            consumer: Some("run:test".into()),
            cleanup: runtime.cleanup.clone(),
        });
        runtime.shutdown().await;
        assert_eq!(
            sources
                .publish(PROJECT_SOURCE_OWNER, &source, json!({"must_not_run":true}))
                .await
                .unwrap_err(),
            "source is passive"
        );
        assert!(runtime.cleanup.lock().unwrap().is_empty());
        sources.shutdown().await;
    }

    #[test]
    fn transform_validation_happens_without_effect_admission() {
        let definition = transform_definition();
        assert!(validate_execution(&definition, None, true).is_err());
        assert!(validate_execution(&definition, Some(&json!({"public":1})), true).is_ok());
        let job = legacy_job(
            &definition,
            super::super::Action::AgentTurn {
                prompt: "test".into(),
                target: super::super::AgentTarget::Isolated,
                mode: None,
                model: None,
                tools: None,
            },
            Trigger::Interval {
                every_ms: 1000,
            },
            "test",
        )
        .unwrap();
        assert_eq!(
            super::super::next_run_ms(&job, job.created_at_ms, super::super::scheduler_timezone()),
            Some(job.created_at_ms + 1000)
        );
    }

    #[test]
    fn transform_selects_only_approved_fields_and_bounds_bytes() {
        let payload = json!({"public":7,"secret":"do not forward"});
        assert_eq!(
            select(&payload, &["public".into()], 64).unwrap(),
            json!({"public":7})
        );
        assert!(select(&payload, &["secret".into()], 2).is_err());
        assert!(matches_filter(&json!({"public":7}), &payload));
        assert!(!matches_filter(&json!({"public":8}), &payload));
        assert!(!matches_filter(&json!("invalid"), &payload));
    }
}
