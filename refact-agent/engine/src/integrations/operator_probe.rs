//! Exact-authorized inert MCP discovery. Initialization and tools/list only; no publication.
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use crate::buddy::resources::{ResourceError, integrations::McpDraft};
use crate::ext::operator_service::{self as files, ResourceRights};
use crate::global_context::GlobalContext;
use refact_buddy_core::operator_types::ActionSpec;
use super::mcp::{
    integr_mcp_common::MCPTransportInitializer,
    session_mcp::{SessionMCP, McpClientHandler, McpRunningService},
};
use super::sessions::IntegrationSession;

pub(crate) type Session = Arc<Mutex<Box<dyn IntegrationSession>>>;

/// One finite budget covers every live candidate and every retirement it still owns: a slot is
/// held from connection until the cleanup finishes, not merely at start.
const SLOTS: u32 = 32;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const LIST_TIMEOUT: Duration = Duration::from_secs(5);
/// Hostile catalogs are cut off by explicit limits instead of accumulating until the timeout.
const LIST_MAX_PAGES: usize = 8;
const LIST_MAX_TOOLS: usize = 256;
const LIST_MAX_PAGE_BYTES: usize = 256 * 1024;
const LIST_MAX_SCHEMA_BYTES: usize = 32 * 1024;

static SLOT_POOL: std::sync::OnceLock<Arc<Semaphore>> = std::sync::OnceLock::new();
struct CleanupState {
    tasks: tokio::task::JoinSet<()>,
    /// A drain briefly closes new handshakes for this GCX, then waits for every already-admitted
    /// candidate to either become a cleanup task or be disarmed into a published session.
    draining: bool,
    candidates: usize,
    /// Retirements that still own a slot. A retirement leaves this count when its future is
    /// dropped, so finishing, being aborted and outliving its drain all report the same way.
    running: usize,
}

struct CleanupOwner {
    state: std::sync::Mutex<CleanupState>,
    changed: tokio::sync::Notify,
    drain: tokio::sync::Mutex<()>,
}

impl CleanupOwner {
    fn locked(&self) -> std::sync::MutexGuard<'_, CleanupState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Park until this owner holds no candidate and no retirement. Every wake follows a real
    /// state change by someone else, so the wait never spins on its own notification.
    async fn settled(&self) {
        loop {
            // Register before examining state so a retirement cannot notify in the gap.
            let changed = self.changed.notified();
            {
                let mut state = self.locked();
                while state.tasks.try_join_next().is_some() {}
                if state.running == 0 && state.candidates == 0 {
                    return;
                }
            }
            changed.await;
        }
    }

    /// Past the deadline the drain stops waiting on candidates and forces every retirement to
    /// its own cancellation point. Retirements leave the owner before they are aborted, so one
    /// that hands its slot to a fresh retirement on the way out is taken over rather than
    /// detached, and no owner lock is ever held while a retirement is being dropped.
    async fn abort_retirements(&self) {
        loop {
            let mut retiring = {
                let mut state = self.locked();
                while state.tasks.try_join_next().is_some() {}
                if state.running == 0 {
                    return;
                }
                std::mem::take(&mut state.tasks)
            };
            retiring.shutdown().await;
        }
    }
}

type OwnerKey = (
    std::sync::Weak<GlobalContext>,
    std::sync::Weak<CleanupOwner>,
);

/// A process-wide registry is only an index from a live GCX to its transient cleanup group.  It
/// holds no cleanup authority: candidates, cleanup tasks, and a current drain own the group.
static CLEANUP_OWNERS: std::sync::LazyLock<std::sync::Mutex<Vec<OwnerKey>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

fn cleanup_owner(gcx: &Arc<GlobalContext>) -> Arc<CleanupOwner> {
    let mut owners = CLEANUP_OWNERS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    owners.retain(|(owner, cleanup)| owner.strong_count() > 0 && cleanup.strong_count() > 0);
    if let Some(index) = owners.iter().position(|(owner, _)| {
        owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, gcx))
    }) {
        if let Some(cleanup) = owners[index].1.upgrade() {
            return cleanup;
        }
        owners.swap_remove(index);
    }
    let cleanup = Arc::new(CleanupOwner {
        state: std::sync::Mutex::new(CleanupState {
            tasks: tokio::task::JoinSet::new(),
            draining: false,
            candidates: 0,
            running: 0,
        }),
        changed: tokio::sync::Notify::const_new(),
        drain: tokio::sync::Mutex::new(()),
    });
    owners.push((Arc::downgrade(gcx), Arc::downgrade(&cleanup)));
    cleanup
}

struct CandidateAdmission {
    cleanup: Arc<CleanupOwner>,
}
impl CandidateAdmission {
    fn acquire(cleanup: Arc<CleanupOwner>) -> Result<Self, ResourceError> {
        let mut state = cleanup.locked();
        if state.draining {
            return Err(ResourceError::ValidationFailed(
                "MCP owner is draining cleanup".into(),
            ));
        }
        state.candidates += 1;
        drop(state);
        Ok(Self {
            cleanup,
        })
    }
}
impl Drop for CandidateAdmission {
    fn drop(&mut self) {
        let mut state = self.cleanup.locked();
        state.candidates = state.candidates.saturating_sub(1);
        drop(state);
        self.cleanup.changed.notify_waiters();
    }
}

/// Closed admission is a scope, not a flag: a drain that is dropped mid-wait reopens its owner
/// instead of rejecting every later candidate for as long as anything keeps the group alive.
struct AdmissionClosed<'a> {
    cleanup: &'a CleanupOwner,
}
impl<'a> AdmissionClosed<'a> {
    fn close(cleanup: &'a CleanupOwner) -> Self {
        cleanup.locked().draining = true;
        cleanup.changed.notify_waiters();
        Self {
            cleanup,
        }
    }
}
impl Drop for AdmissionClosed<'_> {
    fn drop(&mut self) {
        self.cleanup.locked().draining = false;
        self.cleanup.changed.notify_waiters();
    }
}

/// A retirement owns its slot and its place in the running count until its own future is
/// dropped, so a finished, aborted or abandoned retirement always hands the slot back.
struct RunningCleanup {
    cleanup: Arc<CleanupOwner>,
    permit: Option<OwnedSemaphorePermit>,
}
impl Drop for RunningCleanup {
    fn drop(&mut self) {
        self.permit.take();
        let mut state = self.cleanup.locked();
        state.running = state.running.saturating_sub(1);
        drop(state);
        self.cleanup.changed.notify_waiters();
    }
}

fn slots() -> Arc<Semaphore> {
    SLOT_POOL
        .get_or_init(|| Arc::new(Semaphore::new(SLOTS as usize)))
        .clone()
}

fn track_cleanup(
    cleanup: &Arc<CleanupOwner>,
    permit: OwnedSemaphorePermit,
    task: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let mut state = cleanup.locked();
    while state.tasks.try_join_next().is_some() {}
    state.running += 1;
    let running = RunningCleanup {
        cleanup: cleanup.clone(),
        permit: Some(permit),
    };
    state.tasks.spawn(async move {
        let _running = running;
        task.await;
    });
    drop(state);
    cleanup.changed.notify_waiters();
}

/// Bounded wait until this GCX has no candidate or retirement owning a slot; nothing stays
/// detached, and another GCX remains able to admit work while this one shuts down.
pub(crate) async fn drain_cleanup(gcx: &Arc<GlobalContext>) {
    let cleanup = cleanup_owner(gcx);
    // One drain at a time per owner: a second caller waits here instead of racing this group.
    let _drain = cleanup.drain.lock().await;
    // Dropped before the drain lock, so a cancelled drain never leaves admission closed.
    let _closed = AdmissionClosed::close(&cleanup);
    let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;
    if tokio::time::timeout_at(deadline, cleanup.settled())
        .await
        .is_err()
    {
        tracing::warn!("operator MCP cleanup exceeded its shutdown deadline");
        cleanup.abort_retirements().await;
    }
}

pub(crate) struct PreparedMcp {
    pub path: String,
    pub relative: String,
    pub before: Option<Vec<u8>>,
    pub config: serde_json::Value,
    pub after: Vec<u8>,
    pub post: String,
    pub requirement: serde_json::Value,
}

pub(crate) fn prepare(spec: &ActionSpec, draft: &McpDraft) -> Result<PreparedMcp, ResourceError> {
    if spec.adapter != "integrations"
        || !matches!(
            spec.kind.as_str(),
            "mcp_capability_probe" | "mcp_enabled_reload"
        )
        || spec.resource.resource_kind != "mcp"
    {
        return Err(ResourceError::UnsupportedResource);
    }
    if spec.project != draft.project || spec.resource.resource_id != draft.instance_id {
        return Err(ResourceError::DraftDoesNotMatchResource);
    }
    files::validate_name(&draft.instance_id).map_err(ResourceError::ValidationFailed)?;
    let relative = format!("integrations.d/mcp_{}.yaml", draft.instance_id);
    let before = files::snapshot(&spec.project, &relative)?;
    if files::version(&before) != spec.expected_version
        || spec.resource.version != spec.expected_version
    {
        return Err(ResourceError::VersionMismatch);
    }
    let after = super::operator_service::prepare_runtime_config(
        before.as_deref(),
        draft,
        spec.kind == "mcp_enabled_reload",
    )?;
    let mut config: serde_json::Value = serde_yaml::from_slice(&after)
        .map_err(|_| ResourceError::ValidationFailed("invalid MCP config".into()))?;
    if spec.kind == "mcp_enabled_reload" {
        config["operator_action"] = spec.action_id.clone().into();
    }
    let after = serde_yaml::to_string(&config)
        .map(String::into_bytes)
        .map_err(|_| ResourceError::ValidationFailed("invalid MCP config".into()))?;
    let post = if spec.kind == "mcp_capability_probe" {
        spec.expected_version.clone()
    } else {
        files::version(&Some(after.clone()))
    };
    let requirement = serde_json::json!({"operation":spec.kind,"draft":draft,"config_version":spec.expected_version,"restore_previous_effective":true});
    let path = files::checked_path(&spec.project, &relative)?
        .to_string_lossy()
        .into_owned();
    Ok(PreparedMcp {
        path,
        relative,
        before,
        config,
        after,
        post,
        requirement,
    })
}

pub(crate) fn authorize(
    rights: &dyn ResourceRights,
    spec: &ActionSpec,
    draft: &McpDraft,
) -> Result<PreparedMcp, ResourceError> {
    let prepared = prepare(spec, draft)?;
    rights.authorize(spec, &prepared.post, &prepared.requirement)?;
    Ok(prepared)
}

pub(crate) async fn stop(session: &Session) {
    let stop = session.lock().await.try_stop(session.clone());
    Box::into_pin(stop).await;
}

/// Native close boundaries only: rmcp cancel (drops the transport, killing a stdio child) and
/// the session's own `try_stop`; each step is time-bounded.
async fn shutdown(session: Session, client: Option<McpRunningService>) {
    if let Some(client) = client {
        let _ = tokio::time::timeout(CANCEL_TIMEOUT, client.cancel()).await;
    }
    let _ = tokio::time::timeout(CANCEL_TIMEOUT, stop(&session)).await;
}

/// Retire a session inside the finite ownership budget; completes even if the caller is dropped.
pub(crate) async fn retire(
    gcx: &Arc<GlobalContext>,
    session: Session,
    client: Option<McpRunningService>,
) {
    let cleanup = cleanup_owner(gcx);
    match slots().try_acquire_owned() {
        Ok(permit) => {
            let (done, finished) = tokio::sync::oneshot::channel();
            track_cleanup(&cleanup, permit, async move {
                shutdown(session, client).await;
                let _ = done.send(());
            });
            let _ = finished.await;
        }
        Err(_) => shutdown(session, client).await,
    }
}

/// A connected candidate is unpublished and cancellation-owned even when its caller is dropped.
pub(crate) struct Candidate {
    pub session: Session,
    pub armed: bool,
    pub client: Option<McpRunningService>,
    pub tools: Vec<rmcp::model::Tool>,
    permit: Option<OwnedSemaphorePermit>,
    admission: Option<CandidateAdmission>,
    cleanup: Arc<CleanupOwner>,
}
impl Drop for Candidate {
    fn drop(&mut self) {
        let Some(permit) = self.permit.take() else {
            return;
        };
        if !self.armed {
            return;
        }
        let session = self.session.clone();
        let client = self.client.take();
        if tokio::runtime::Handle::try_current().is_ok() {
            track_cleanup(&self.cleanup, permit, shutdown(session, client));
        }
    }
}
impl Candidate {
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
        self.permit.take();
        self.admission.take();
    }
}

pub(crate) fn restricted_handler(
    gcx: std::sync::Weak<GlobalContext>,
    session: &Session,
    path: &str,
) -> McpClientHandler {
    McpClientHandler {
        restricted: true,
        peer_arc: Arc::new(Mutex::new(None)),
        session_arc: session.clone(),
        logs: Arc::new(Mutex::new(Vec::new())),
        debug_name: path.into(),
        request_timeout: LIST_TIMEOUT.as_secs(),
        gcx,
        tool_refresh_handle: Arc::new(Mutex::new(None)),
        resource_refresh_handle: Arc::new(Mutex::new(None)),
        prompt_refresh_handle: Arc::new(Mutex::new(None)),
    }
}

/// tools/list only, bounded by pages, tools, page bytes and schema bytes; a repeated cursor or
/// any granted tool missing from the catalog fails the listing. Returns exactly the scoped tools.
pub(crate) async fn list_scoped_tools(
    client: &McpRunningService,
    scope: &[String],
) -> Result<Vec<rmcp::model::Tool>, ResourceError> {
    let fail = |reason: &str| ResourceError::ValidationFailed(format!("MCP tools/list {reason}"));
    let mut seen_cursors = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    let mut catalog: Vec<rmcp::model::Tool> = Vec::new();
    for _ in 0..LIST_MAX_PAGES {
        let page = client
            .list_tools(Some(
                rmcp::model::PaginatedRequestParams::default().with_cursor(cursor.clone()),
            ))
            .await
            .map_err(|_| fail("failed"))?;
        let bytes = serde_json::to_vec(&page).map_err(|_| fail("failed"))?;
        if bytes.len() > LIST_MAX_PAGE_BYTES {
            return Err(fail("page exceeds size limit"));
        }
        for tool in page.tools {
            if catalog.len() >= LIST_MAX_TOOLS {
                return Err(fail("exceeds tool count limit"));
            }
            let schema = serde_json::to_vec(&tool.input_schema).map_err(|_| fail("failed"))?;
            if schema.len() > LIST_MAX_SCHEMA_BYTES {
                return Err(fail("tool schema exceeds size limit"));
            }
            catalog.push(tool);
        }
        cursor = page.next_cursor;
        let Some(next) = &cursor else {
            let scoped: Vec<rmcp::model::Tool> = scope
                .iter()
                .filter_map(|name| catalog.iter().find(|t| t.name.as_ref() == name).cloned())
                .collect();
            if scoped.len() != scope.len() {
                return Err(fail("catalog lacks granted tools"));
            }
            return Ok(scoped);
        };
        if next.is_empty() || next.len() > 4096 {
            return Err(fail("invalid cursor"));
        }
        if !seen_cursors.insert(next.clone()) {
            return Err(fail("repeated cursor"));
        }
    }
    Err(fail("exceeds page limit"))
}

pub(crate) async fn connect(
    gcx: Arc<GlobalContext>,
    path: &str,
    config: serde_json::Value,
) -> Result<Candidate, ResourceError> {
    use super::mcp::integr_mcp_stdio::IntegrationMCPStdio;
    if gcx.shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
        return Err(ResourceError::ValidationFailed(
            "MCP owner is shutting down".into(),
        ));
    }
    let scope = serde_json::from_value::<Vec<String>>(config["operator_tools"].clone())
        .ok()
        .filter(|scope| files::exact_names(scope))
        .ok_or_else(|| ResourceError::ValidationFailed("invalid operator tool scope".into()))?;
    let cleanup = cleanup_owner(&gcx);
    let admission = CandidateAdmission::acquire(cleanup.clone())?;
    let permit = slots()
        .try_acquire_owned()
        .map_err(|_| ResourceError::ValidationFailed("MCP connection capacity reached".into()))?;
    let session: Session = Arc::new(Mutex::new(Box::new(SessionMCP::detached(path))));
    let handler = restricted_handler(Arc::downgrade(&gcx), &session, path);
    let invalid = || ResourceError::ValidationFailed("MCP connection or listing failed".into());
    let transport: Box<dyn MCPTransportInitializer> = if config.get("command").is_some() {
        Box::new(IntegrationMCPStdio {
            cfg: serde_json::from_value(config.clone()).map_err(|_| invalid())?,
            config_path: path.into(),
            gcx_option: Some(Arc::downgrade(&gcx)),
            ..Default::default()
        })
    } else {
        Box::new(ExactHttp {
            config: config.clone(),
            path: path.into(),
        })
    };
    let mut candidate = Candidate {
        session,
        armed: true,
        client: None,
        tools: vec![],
        permit: Some(permit),
        admission: Some(admission),
        cleanup,
    };
    candidate.client = Some(
        tokio::time::timeout(
            CONNECT_TIMEOUT,
            transport.init_mcp_transport(
                handler.logs.clone(),
                path.into(),
                LIST_TIMEOUT.as_secs(),
                LIST_TIMEOUT.as_secs(),
                candidate.session.clone(),
                handler,
            ),
        )
        .await
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?,
    );
    candidate.tools = tokio::time::timeout(
        LIST_TIMEOUT,
        list_scoped_tools(candidate.client.as_ref().unwrap(), &scope),
    )
    .await
    .map_err(|_| invalid())??;
    candidate
        .session
        .lock()
        .await
        .as_any_mut()
        .downcast_mut::<SessionMCP>()
        .unwrap()
        .launched_cfg = config;
    Ok(candidate)
}

/// Only the selected native Streamable HTTP destination with protocol headers and no auth:
/// the shared transport never follows redirects, never retries with different rights and only
/// reports a 401 as a failure (OAuth discovery lives in `mcp_session_setup`, never reached here).
struct ExactHttp {
    config: serde_json::Value,
    path: String,
}
#[async_trait::async_trait]
impl MCPTransportInitializer for ExactHttp {
    async fn init_mcp_transport(
        &self,
        logs: Arc<Mutex<Vec<String>>>,
        debug_name: String,
        init_timeout: u64,
        _: u64,
        session: Session,
        handler: McpClientHandler,
    ) -> Option<McpRunningService> {
        use super::mcp::{integr_mcp_http, mcp_auth::AuthType};
        let cfg: integr_mcp_http::SettingsMCPHttp =
            serde_json::from_value(self.config.clone()).ok()?;
        if cfg.auth.auth_type != AuthType::None
            || !cfg.auth.bearer_token.is_empty()
            || self
                .config
                .get("headers")
                .is_some_and(|h| h != &serde_json::json!({}))
        {
            return None;
        }
        super::mcp::integr_mcp_common::serve_streamable_http_transport(
            cfg.mcp_url,
            integr_mcp_http::default_http_headers(),
            cfg.auth,
            self.path.clone(),
            "Operator HTTP",
            init_timeout,
            logs,
            debug_name,
            session,
            handler,
        )
        .await
    }
}

pub async fn probe_mcp(
    gcx: Arc<GlobalContext>,
    rights: &dyn ResourceRights,
    spec: &ActionSpec,
    draft: &McpDraft,
) -> Result<Vec<String>, ResourceError> {
    if spec.kind != "mcp_capability_probe" {
        return Err(ResourceError::UnsupportedResource);
    }
    let prepared = authorize(rights, spec, draft)?;
    let mut candidate = connect(gcx, &prepared.path, prepared.config).await?;
    authorize(rights, spec, draft)?;
    let names = candidate.tools.iter().map(|t| t.name.to_string()).collect();
    let client = candidate.client.take();
    let permit = candidate.permit.take();
    let session = candidate.session.clone();
    let (done, finished) = tokio::sync::oneshot::channel();
    track_cleanup(
        &candidate.cleanup,
        permit.expect("connected candidate owns a slot"),
        async move {
            shutdown(session, client).await;
            let _ = done.send(());
        },
    );
    candidate.disarm();
    let _ = finished.await;
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// Scratch line-delimited MCP server: initialize plus cursor-driven tools/list pages.
    async fn scoped_listing(
        pages: impl Fn(Option<&str>) -> serde_json::Value + Send + 'static,
        scope: &[&str],
    ) -> Result<Vec<String>, ResourceError> {
        let (client_io, server_io) = tokio::io::duplex(1 << 20);
        let server = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server_io);
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let result = match request["method"].as_str() {
                    Some("initialize") => {
                        serde_json::json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"scratch","version":"1"}})
                    }
                    Some("tools/list") => pages(request["params"]["cursor"].as_str()),
                    _ => continue,
                };
                let response =
                    serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result});
                if write
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let session: Session = Arc::new(Mutex::new(Box::new(SessionMCP::detached("scratch"))));
        let handler = restricted_handler(std::sync::Weak::new(), &session, "scratch");
        let client = tokio::time::timeout(LIST_TIMEOUT, rmcp::serve_client(handler, client_io))
            .await
            .unwrap()
            .unwrap();
        let scope: Vec<String> = scope.iter().map(|s| s.to_string()).collect();
        let listed = tokio::time::timeout(LIST_TIMEOUT, list_scoped_tools(&client, &scope))
            .await
            .unwrap();
        let _ = tokio::time::timeout(CANCEL_TIMEOUT, client.cancel()).await;
        server.abort();
        let _ = server.await;
        listed.map(|tools| tools.iter().map(|t| t.name.to_string()).collect())
    }

    fn page(names: &[&str], next: Option<&str>) -> serde_json::Value {
        let tools: Vec<_> = names
            .iter()
            .map(|n| serde_json::json!({"name":n,"inputSchema":{"type":"object"}}))
            .collect();
        match next {
            Some(next) => serde_json::json!({"tools":tools,"nextCursor":next}),
            None => serde_json::json!({"tools":tools}),
        }
    }

    #[tokio::test]
    async fn listing_is_bounded_exact_and_rejects_hostile_catalogs() {
        assert_eq!(
            scoped_listing(
                |cursor| match cursor {
                    None => page(&["extra", "write"], Some("a")),
                    Some("a") => page(&["read"], Some("b")),
                    _ => page(&["other"], None),
                },
                &["read", "write"],
            )
            .await
            .unwrap(),
            vec!["read", "write"]
        );
        let reason = |result: Result<Vec<String>, ResourceError>| match result {
            Err(ResourceError::ValidationFailed(reason)) => reason,
            other => panic!("expected bounded failure, got {other:?}"),
        };
        assert!(
            reason(scoped_listing(|_| page(&["read"], Some("loop")), &["read"]).await)
                .contains("repeated cursor")
        );
        assert!(
            reason(scoped_listing(|_| page(&["read"], None), &["read", "write"]).await)
                .contains("lacks granted tools")
        );
        let counter = std::sync::atomic::AtomicUsize::new(0);
        assert!(reason(
            scoped_listing(
                move |_| {
                    let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    page(&["read"], Some(&format!("c{n}")))
                },
                &["read"],
            )
            .await
        )
        .contains("page limit"));
        let many: Vec<String> = (0..LIST_MAX_TOOLS + 1).map(|i| format!("t{i}")).collect();
        assert!(reason(
            scoped_listing(
                move |_| page(&many.iter().map(String::as_str).collect::<Vec<_>>(), None),
                &["t0"],
            )
            .await
        )
        .contains("tool count limit"));
        assert!(reason(
            scoped_listing(
                |_| serde_json::json!({"tools":[{"name":"read","inputSchema":{"type":"object","description":"x".repeat(LIST_MAX_SCHEMA_BYTES + 1)}}]}),
                &["read"],
            )
            .await
        )
        .contains("schema exceeds"));
        assert!(reason(
            scoped_listing(
                |_| serde_json::json!({"tools":[{"name":"read","description":"x".repeat(LIST_MAX_PAGE_BYTES + 1),"inputSchema":{"type":"object"}}]}),
                &["read"],
            )
            .await
        )
        .contains("page exceeds"));
    }

    /// Scratch HTTP listeners: the selected endpoint answers with a redirect, then with a 401
    /// carrying OAuth discovery metadata; the redirect/metadata target must never be contacted
    /// and the connect must fail closed before any publication.
    #[tokio::test]
    async fn exact_http_never_follows_redirects_or_auth_discovery() {
        use tokio::io::AsyncReadExt;
        let forbidden = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let forbidden_port = forbidden.local_addr().unwrap().port();
        let forbidden_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let forbidden_task = tokio::spawn({
            let hits = forbidden_hits.clone();
            async move {
                while forbidden.accept().await.is_ok() {
                    hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
        let endpoint = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint_port = endpoint.local_addr().unwrap().port();
        let endpoint_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let endpoint_task = tokio::spawn({
            let hits = endpoint_hits.clone();
            async move {
                let responses = [
                    format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{forbidden_port}/mcp\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    format!("HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer resource_metadata=\"http://127.0.0.1:{forbidden_port}/.well-known/oauth-protected-resource\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                ];
                let mut index = 0;
                while let Ok((mut socket, _)) = endpoint.accept().await {
                    let mut buffer = vec![0u8; 8192];
                    let _ = socket.read(&mut buffer).await;
                    let request = String::from_utf8_lossy(&buffer);
                    assert!(request.starts_with("POST /mcp "), "{request}");
                    assert!(!request.to_ascii_lowercase().contains("authorization:"));
                    let _ = socket.write_all(responses[index.min(1)].as_bytes()).await;
                    index += 1;
                    hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config = |extra: serde_json::Value| {
            let mut config = serde_json::json!({"url":format!("http://127.0.0.1:{endpoint_port}/mcp"),"operator_tools":["read"]});
            for (k, v) in extra.as_object().unwrap() {
                config[k] = v.clone();
            }
            config
        };
        for forbidden_config in [
            serde_json::json!({"auth_type":"bearer","bearer_token":"secret"}),
            serde_json::json!({"headers":{"Authorization":"Bearer secret"}}),
        ] {
            assert!(
                connect(gcx.clone(), "/scratch/exact.yaml", config(forbidden_config))
                    .await
                    .is_err()
            );
        }
        assert_eq!(endpoint_hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        for _ in 0..2 {
            assert!(connect(
                gcx.clone(),
                "/scratch/exact.yaml",
                config(serde_json::json!({}))
            )
            .await
            .is_err());
        }
        drain_cleanup(&gcx).await;
        assert!(endpoint_hits.load(std::sync::atomic::Ordering::SeqCst) >= 2);
        assert_eq!(forbidden_hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(gcx.integration_sessions.lock().await.is_empty());
        endpoint_task.abort();
        forbidden_task.abort();
        let _ = endpoint_task.await;
        let _ = forbidden_task.await;
    }

    #[tokio::test]
    async fn drain_closes_admission_and_waits_for_dropped_candidate_cleanup() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let cleanup = cleanup_owner(&gcx);
        let pool = Arc::new(Semaphore::new(1));
        let candidate = Candidate {
            session: Arc::new(Mutex::new(Box::new(SessionMCP::detached(
                "/scratch/candidate",
            )))),
            armed: true,
            client: None,
            tools: vec![],
            permit: Some(pool.clone().try_acquire_owned().unwrap()),
            admission: Some(CandidateAdmission::acquire(cleanup.clone()).unwrap()),
            cleanup: cleanup.clone(),
        };
        let mut drain = Box::pin(drain_cleanup(&gcx));
        assert!(futures::poll!(drain.as_mut()).is_pending());
        assert_eq!(cleanup.state.lock().unwrap().candidates, 1);
        assert!(CandidateAdmission::acquire(cleanup.clone()).is_err());
        drop(candidate);
        drain.await;
        let state = cleanup.state.lock().unwrap();
        assert_eq!(state.candidates, 0);
        assert_eq!(state.running, 0);
        assert!(state.tasks.is_empty());
        assert!(!state.draining);
        assert_eq!(pool.available_permits(), 1);
    }

    #[tokio::test]
    async fn cancelled_drain_reopens_admission_without_detaching_retirement() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let cleanup = cleanup_owner(&gcx);
        let pool = Arc::new(Semaphore::new(1));
        let (release, retired) = tokio::sync::oneshot::channel();
        track_cleanup(
            &cleanup,
            pool.clone().try_acquire_owned().unwrap(),
            async move {
                let _ = retired.await;
            },
        );
        let mut drain = Box::pin(drain_cleanup(&gcx));
        assert!(futures::poll!(drain.as_mut()).is_pending());
        assert!(CandidateAdmission::acquire(cleanup.clone()).is_err());
        drop(drain);
        let admitted = CandidateAdmission::acquire(cleanup.clone()).unwrap();
        assert_eq!(cleanup.state.lock().unwrap().running, 1);
        assert_eq!(pool.available_permits(), 0);
        drop(admitted);
        release.send(()).unwrap();
        drain_cleanup(&gcx).await;
        assert_eq!(cleanup.state.lock().unwrap().running, 0);
        assert_eq!(pool.available_permits(), 1);
        assert!(!cleanup.state.lock().unwrap().draining);
    }

    #[tokio::test]
    async fn cancelled_queued_drain_does_not_reopen_current_owners_admission() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let cleanup = cleanup_owner(&gcx);
        let admitted = CandidateAdmission::acquire(cleanup.clone()).unwrap();
        let mut first = Box::pin(drain_cleanup(&gcx));
        assert!(futures::poll!(first.as_mut()).is_pending());
        let mut queued = Box::pin(drain_cleanup(&gcx));
        assert!(futures::poll!(queued.as_mut()).is_pending());
        drop(queued);
        assert!(cleanup.state.lock().unwrap().draining);
        assert!(CandidateAdmission::acquire(cleanup.clone()).is_err());
        drop(admitted);
        first.await;
        assert!(!cleanup.state.lock().unwrap().draining);
        assert!(CandidateAdmission::acquire(cleanup.clone()).is_ok());
    }

    #[tokio::test]
    async fn retirement_retains_owner_until_completion_then_releases_weak_index() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let cleanup = cleanup_owner(&gcx);
        let weak = Arc::downgrade(&cleanup);
        let pool = Arc::new(Semaphore::new(1));
        let (release, retired) = tokio::sync::oneshot::channel();
        track_cleanup(
            &cleanup,
            pool.clone().try_acquire_owned().unwrap(),
            async move {
                let _ = retired.await;
            },
        );
        drop(cleanup);
        assert!(weak.upgrade().is_some());
        assert_eq!(pool.available_permits(), 0);
        release.send(()).unwrap();
        drain_cleanup(&gcx).await;
        assert_eq!(pool.available_permits(), 1);
        assert!(weak.upgrade().is_none());
        gcx.shutdown_flag
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(matches!(
            connect(gcx, "/not-used.yaml", serde_json::json!({})).await,
            Err(ResourceError::ValidationFailed(reason)) if reason == "MCP owner is shutting down"
        ));
    }

    #[tokio::test]
    async fn drain_scopes_candidate_admission_and_cleanup_to_its_gcx() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let other = crate::global_context::tests::make_test_gcx().await;
        let cleanup = cleanup_owner(&gcx);
        let other_cleanup = cleanup_owner(&other);
        let admission = CandidateAdmission::acquire(cleanup.clone()).unwrap();
        let own_pool = Arc::new(Semaphore::new(1));
        let other_pool = Arc::new(Semaphore::new(1));
        let (release_own, wait_for_own) = tokio::sync::oneshot::channel();
        let (release_other, wait_for_other) = tokio::sync::oneshot::channel();
        track_cleanup(
            &cleanup,
            own_pool.clone().try_acquire_owned().unwrap(),
            async move {
                let _ = wait_for_own.await;
            },
        );
        track_cleanup(
            &other_cleanup,
            other_pool.clone().try_acquire_owned().unwrap(),
            async move {
                let _ = wait_for_other.await;
            },
        );
        let draining = tokio::spawn(async move { drain_cleanup(&gcx).await });
        loop {
            let changed = cleanup.changed.notified();
            if cleanup.state.lock().unwrap().draining {
                break;
            }
            changed.await;
        }
        assert!(matches!(
            CandidateAdmission::acquire(cleanup.clone()),
            Err(ResourceError::ValidationFailed(reason)) if reason == "MCP owner is draining cleanup"
        ));
        let other_admission = CandidateAdmission::acquire(other_cleanup.clone()).unwrap();
        assert!(
            !draining.is_finished(),
            "drain returned while its owner still had a candidate or cleanup task"
        );
        drop(admission);
        release_own.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), draining)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(own_pool.available_permits(), 1);
        assert_eq!(other_pool.available_permits(), 0);
        drop(other_admission);
        release_other.send(()).unwrap();
        drain_cleanup(&other).await;
        assert_eq!(other_pool.available_permits(), 1);
    }

    /// Two drains for one owner never overlap: the queued one waits for the live one instead of
    /// racing it, and the owner is open again between them and after them.
    #[tokio::test]
    async fn same_owner_drains_run_one_at_a_time() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let cleanup = cleanup_owner(&gcx);
        let pool = Arc::new(Semaphore::new(1));
        let (release, retired) = tokio::sync::oneshot::channel();
        track_cleanup(
            &cleanup,
            pool.clone().try_acquire_owned().unwrap(),
            async move {
                let _ = retired.await;
            },
        );
        let mut first = Box::pin(drain_cleanup(&gcx));
        let mut queued = Box::pin(drain_cleanup(&gcx));
        assert!(futures::poll!(first.as_mut()).is_pending());
        assert!(futures::poll!(queued.as_mut()).is_pending());
        assert!(
            cleanup.drain.try_lock().is_err(),
            "a queued drain must not run beside the live one"
        );
        release.send(()).unwrap();
        first.await;
        assert_eq!(pool.available_permits(), 1);
        assert!(CandidateAdmission::acquire(cleanup.clone()).is_ok());
        queued.await;
        assert!(!cleanup.state.lock().unwrap().draining);
        assert!(cleanup.drain.try_lock().is_ok());
    }

    /// Stands in for a candidate dropped while a drain is already past its deadline: exactly like
    /// `Candidate::drop`, it hands its slot to a fresh retirement on the way out.
    struct HandOver {
        cleanup: Arc<CleanupOwner>,
        permit: Option<OwnedSemaphorePermit>,
    }
    impl Drop for HandOver {
        fn drop(&mut self) {
            let Some(permit) = self.permit.take() else {
                return;
            };
            if tokio::runtime::Handle::try_current().is_ok() {
                track_cleanup(&self.cleanup, permit, std::future::pending());
            }
        }
    }

    /// A drain that runs out of time still finishes the job: it aborts the retirement it was
    /// waiting on, takes over the slot that retirement hands back as it unwinds, and leaves the
    /// owner open. Fake time also proves the wait parks instead of spinning on its own wake-ups.
    #[tokio::test(start_paused = true)]
    async fn drain_past_its_deadline_aborts_retirements_and_their_handovers() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let cleanup = cleanup_owner(&gcx);
        let pool = Arc::new(Semaphore::new(2));
        let handover = HandOver {
            cleanup: cleanup.clone(),
            permit: Some(pool.clone().try_acquire_owned().unwrap()),
        };
        track_cleanup(
            &cleanup,
            pool.clone().try_acquire_owned().unwrap(),
            async move {
                let _handover = handover;
                std::future::pending::<()>().await;
            },
        );
        let stuck = CandidateAdmission::acquire(cleanup.clone()).unwrap();
        let started = tokio::time::Instant::now();
        drain_cleanup(&gcx).await;
        assert_eq!(started.elapsed(), DRAIN_TIMEOUT);
        assert_eq!(
            pool.available_permits(),
            2,
            "a drain past its deadline still returns every slot it owned"
        );
        let state = cleanup.state.lock().unwrap();
        assert_eq!(state.running, 0);
        assert!(state.tasks.is_empty());
        assert!(!state.draining);
        drop(state);
        drop(stuck);
        assert!(CandidateAdmission::acquire(cleanup).is_ok());
    }
}
