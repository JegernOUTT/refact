use super::*;
use hyper::body::Bytes;
use refact_browser::project_frame_stream::{
    Frame, ProjectFrameStream, StreamFailure, StreamIdentity, Viewport, MAX_JPEG_BYTES,
};
use refact_browser::screencast::ScreencastBusy;
use serde::Serialize;
use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tokio::sync::watch;

const CONTENT_TYPE: &str = "application/x-refact-browser-frames";
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_FRAME_URL_JSON_BYTES: usize = 24 * 1024;
const MAX_FRAME_TITLE_BYTES: usize = 1024;
const HEARTBEAT: Duration = Duration::from_secs(10);
const PANE_SYNC_INTERVAL: Duration = Duration::from_millis(500);
const VIEWER_RECONNECT_GRACE: Duration = Duration::from_secs(3);
const VIEWER_RECONNECT_POLL: Duration = Duration::from_millis(25);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscribeBody {
    tab_id: String,
    runtime_id: String,
    max_side: Option<u32>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Record<'a> {
    Frame {
        runtime_id: &'a str,
        tab_id: &'a str,
        mime: &'static str,
        width: u32,
        height: u32,
        url: &'a str,
        title: &'a str,
        viewport: Viewport,
    },
    Heartbeat,
    Error {
        status: u16,
        detail: &'static str,
    },
}

struct HeaderBuffer(Vec<u8>);

impl std::io::Write for HeaderBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_HEADER_BYTES - self.0.len() {
            return Err(std::io::Error::other(
                "frame header exceeded its byte limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encode(header: Record<'_>, payload: &[u8]) -> Option<Bytes> {
    if payload.len() > MAX_JPEG_BYTES
        || matches!(header, Record::Frame { .. }) == payload.is_empty()
    {
        return None;
    }
    let mut json = HeaderBuffer(Vec::new());
    serde_json::to_writer(&mut json, &header).ok()?;
    let mut record = Vec::with_capacity(8 + json.0.len() + payload.len());
    record.extend_from_slice(&(json.0.len() as u32).to_be_bytes());
    record.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    record.extend_from_slice(&json.0);
    record.extend_from_slice(payload);
    Some(Bytes::from(record))
}

fn encode_frame(frame: &Frame, runtime_id: &str, tab_id: &str) -> Option<Bytes> {
    let url = if frame.url.len() <= MAX_FRAME_URL_JSON_BYTES
        && serde_json::to_vec(&frame.url).ok()?.len() <= MAX_FRAME_URL_JSON_BYTES
    {
        frame.url.as_str()
    } else {
        ""
    };
    let title = if frame.title.len() <= MAX_FRAME_TITLE_BYTES {
        std::borrow::Cow::Borrowed(frame.title.as_str())
    } else {
        let end = frame
            .title
            .floor_char_boundary(MAX_FRAME_TITLE_BYTES - '…'.len_utf8());
        std::borrow::Cow::Owned(format!("{}…", &frame.title[..end]))
    };
    encode(
        Record::Frame {
            runtime_id,
            tab_id,
            mime: "image/jpeg",
            width: frame.width,
            height: frame.height,
            url,
            title: &title,
            viewport: frame.viewport,
        },
        &frame.jpeg,
    )
}

fn encode_error(failure: StreamFailure) -> Option<Bytes> {
    let (status, detail) = match failure {
        StreamFailure::Replaced => (409, "the project browser was replaced or reconnected"),
        StreamFailure::Closed => (410, "stale tab_id: no such tab"),
        StreamFailure::ScopeChanged => (409, "the current project changed"),
        StreamFailure::Unavailable => (502, "the browser frame stream became unavailable"),
        StreamFailure::Stopped => return None,
    };
    encode(
        Record::Error {
            status,
            detail,
        },
        &[],
    )
}

struct ScopeFence {
    folders: Arc<Mutex<Vec<PathBuf>>>,
    selected: Vec<PathBuf>,
}

impl ScopeFence {
    fn new(app: &AppState) -> Self {
        let folders = app.gcx.documents_state.workspace_folders.clone();
        let selected = folders
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        Self {
            folders,
            selected,
        }
    }

    fn check(&self) -> Result<(), StreamFailure> {
        if self
            .folders
            .lock()
            .is_ok_and(|folders| *folders == self.selected)
        {
            Ok(())
        } else {
            Err(StreamFailure::ScopeChanged)
        }
    }
}

struct StreamGuard {
    identity: StreamIdentity,
    scope: Option<ScopeFence>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

impl StreamGuard {
    fn check(&self) -> Result<(), StreamFailure> {
        if self.shutdown.load(Ordering::Relaxed) {
            return Err(StreamFailure::Stopped);
        }
        if let Some(scope) = &self.scope {
            scope.check()?;
        }
        self.identity.check()
    }
}

#[derive(Clone)]
enum Update {
    Pending,
    Frame(Arc<Frame>),
    Terminal(StreamFailure),
}

async fn reserve_after_viewer<T, F>(mut reserve: impl FnMut() -> F) -> Result<T, ScratchError>
where
    F: Future<Output = Result<Result<T, ScreencastBusy>, ScratchError>>,
{
    let conflict = |busy: ScreencastBusy| fail(StatusCode::CONFLICT, busy.to_string());
    match reserve().await? {
        Ok(stream) => return Ok(stream),
        Err(ScreencastBusy::Tool) => return Err(conflict(ScreencastBusy::Tool)),
        Err(ScreencastBusy::ProjectView) => {}
    }

    tokio::time::timeout(VIEWER_RECONNECT_GRACE, async {
        loop {
            tokio::time::sleep(VIEWER_RECONNECT_POLL).await;
            match reserve().await? {
                Ok(stream) => return Ok(stream),
                Err(ScreencastBusy::Tool) => return Err(conflict(ScreencastBusy::Tool)),
                Err(ScreencastBusy::ProjectView) => {}
            }
        }
    })
    .await
    .map_err(|_| conflict(ScreencastBusy::ProjectView))?
}

fn sync_pending_pane(runtime: &RuntimeArc, fence: &TabFence, tab_gate: &Arc<TabGate>) {
    let Ok(gate) = tab_gate.acquire(GateUse::Resize, Instant::now()) else {
        return;
    };
    let tab_id = fence.tab.get_target_id();
    let size = {
        let Ok(rt) = runtime.try_lock() else { return };
        if fence.validate(&rt, "").is_err() {
            return;
        }
        pane_to_apply(&rt, &gate, tab_id)
    };
    let Some(size) = size else { return };
    let _deadline = headless_chrome::browser::transport::CommandDeadlineGuard::new(
        Instant::now() + PANE_SYNC_INTERVAL,
    );
    if apply_pane_viewport(&fence.tab, size).is_ok() {
        if let Ok(mut rt) = runtime.try_lock() {
            if fence.validate(&rt, "").is_ok() {
                rt.pane.keep_tab_size(tab_id, size);
                gate.update_view(|view| view.applied_pane = Some(size));
            }
        }
    }
}

pub(super) async fn subscribe(
    State(app): State<AppState>,
    body: Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: SubscribeBody = parse_body(&body)?;
    if post.tab_id.trim().is_empty() || post.runtime_id.trim().is_empty() {
        return Err(unprocessable(
            "runtime_id and tab_id must be nonempty".into(),
        ));
    }
    let max_side = post
        .max_side
        .unwrap_or(DEFAULT_FRAME_SIDE)
        .clamp(MIN_FRAME_SIDE, MAX_FRAME_SIDE);
    let scope_fence = pinned_project_scope(&app)
        .is_none()
        .then(|| ScopeFence::new(&app));
    let scope = scope(&app).await?;
    let check_scope = || {
        scope_fence
            .as_ref()
            .map_or(Ok(()), ScopeFence::check)
            .map_err(|_| fail(StatusCode::CONFLICT, "the current project changed"))
    };
    check_scope()?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    let (tab, tab_gate, fence) = {
        let mut rt = lock_runtime(&runtime).await?;
        let tab = tab_by_id(&rt, &post.tab_id);
        check_fence(&post.runtime_id, &runtime_id, tab.is_some()).map_err(fence_error)?;
        let tab = tab.ok_or_else(stale_tab)?;
        let gate = rt.tab_gate(&post.tab_id);
        let fence = TabFence::new(&rt, tab.clone());
        (tab, gate, fence)
    };
    let stream = reserve_after_viewer(|| async {
        check_scope()?;
        let rt = fence.lock(&runtime, "").await?;
        check_scope()?;
        Ok(ProjectFrameStream::reserve(
            tab.clone(),
            rt.ownership.clone(),
            rt.world_manager.clone(),
        ))
    })
    .await?;
    let gate = take_gate(&tab_gate, GateUse::Frame, FRAME_SKIPPED)?;
    let pane_size = {
        let rt = fence.lock(&runtime, "").await?;
        pane_to_apply(&rt, &gate, &post.tab_id)
    };
    if let Some(size) = pane_size {
        check_scope()?;
        blocking_within(RENDERER_DEADLINE, || apply_pane_viewport(&tab, size))
            .map_err(|failure| renderer_failure_status(failure, &RESIZE_FAILURE))?;
        let mut rt = fence.lock(&runtime, "").await?;
        rt.pane.keep_tab_size(&post.tab_id, size);
        gate.update_view(|view| view.applied_pane = Some(size));
    }
    drop(gate);
    let guard = Arc::new(StreamGuard {
        identity: stream.identity(),
        scope: scope_fence,
        shutdown: app.gcx.shutdown_flag.clone(),
    });
    guard.check().map_err(|failure| match failure {
        StreamFailure::Closed => stale_tab(),
        StreamFailure::Replaced => fence.replaced(""),
        StreamFailure::ScopeChanged => fail(StatusCode::CONFLICT, "the current project changed"),
        _ => fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "the browser frame stream is unavailable",
        ),
    })?;

    let (updates, mut receive) = watch::channel(Update::Pending);
    let worker_guard = guard.clone();
    let worker = tokio::task::spawn_blocking(move || {
        let mut next_pane_sync = Instant::now() + PANE_SYNC_INTERVAL;
        let result = stream.run(
            u32::from(FRAME_JPEG_QUALITY),
            max_side,
            || {
                if updates.is_closed() {
                    return Err(StreamFailure::Stopped);
                }
                worker_guard.check()
            },
            || {
                if Instant::now() >= next_pane_sync {
                    sync_pending_pane(&runtime, &fence, &tab_gate);
                    next_pane_sync = Instant::now() + PANE_SYNC_INTERVAL;
                }
            },
            |frame| {
                updates
                    .send(Update::Frame(Arc::new(frame)))
                    .map_err(|_| StreamFailure::Stopped)
            },
        );
        if let Err(failure) = result {
            let _ = updates.send(Update::Terminal(failure));
        }
    });
    let body = async_stream::stream! {
        let _worker = worker;
        loop {
            if let Err(failure) = guard.check() {
                if let Some(bytes) = encode_error(failure) {
                    yield Ok::<_, Infallible>(bytes);
                }
                break;
            }
            let changed = tokio::time::timeout(HEARTBEAT, receive.changed()).await;
            // Never release a buffered record once its tab, project, or runtime retired.
            if let Err(failure) = guard.check() {
                if let Some(bytes) = encode_error(failure) {
                    yield Ok(bytes);
                }
                break;
            }
            let update = match changed {
                Ok(Ok(())) => receive.borrow_and_update().clone(),
                Ok(Err(_)) => break,
                Err(_) => {
                    yield Ok(encode(Record::Heartbeat, &[]).expect("fixed heartbeat"));
                    continue;
                }
            };
            match update {
                Update::Frame(frame) if guard.identity.is_current_frame(frame.revision) => {
                    let Some(record) = encode_frame(&frame, &runtime_id, &post.tab_id) else {
                        drop(receive);
                        if let Some(bytes) = encode_error(StreamFailure::Unavailable) {
                            yield Ok(bytes);
                        }
                        break;
                    };
                    if let Err(failure) = guard.check() {
                        if let Some(bytes) = encode_error(failure) {
                            yield Ok(bytes);
                        }
                        break;
                    }
                    if guard.identity.is_current_frame(frame.revision) {
                        yield Ok(record);
                    }
                }
                Update::Terminal(failure) => {
                    if let Some(bytes) = encode_error(failure) {
                        yield Ok(bytes);
                    }
                    break;
                }
                _ => {}
            }
        }
    };
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, CONTENT_TYPE)
        .body(Body::wrap_stream(body))
        .expect("fixed stream response headers"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test(start_paused = true)]
    async fn reconnect_admission_waits_for_the_prior_viewer_to_release() {
        let state = Cell::new(Some(ScreencastBusy::ProjectView));
        let attempts = Cell::new(0);
        let admission = reserve_after_viewer(|| {
            attempts.set(attempts.get() + 1);
            std::future::ready(Ok(state.get().map_or(Ok(7), Err)))
        });
        tokio::pin!(admission);
        assert!(futures::poll!(&mut admission).is_pending());
        assert_eq!(attempts.get(), 1);
        state.set(None);
        tokio::time::advance(VIEWER_RECONNECT_POLL).await;
        assert_eq!(admission.await.unwrap(), 7);
        assert_eq!(attempts.get(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_admission_cannot_displace_an_active_viewer_or_wait_for_a_tool() {
        let now = tokio::time::Instant::now();
        let tool = reserve_after_viewer(|| {
            std::future::ready(Ok::<_, ScratchError>(Err::<(), _>(ScreencastBusy::Tool)))
        })
        .await
        .unwrap_err();
        assert_eq!(tokio::time::Instant::now(), now);
        assert_eq!(tool.status_code, StatusCode::CONFLICT);
        assert_eq!(tool.message, ScreencastBusy::Tool.to_string());

        let viewer = reserve_after_viewer(|| {
            std::future::ready(Ok::<_, ScratchError>(Err::<(), _>(
                ScreencastBusy::ProjectView,
            )))
        })
        .await
        .unwrap_err();
        assert!(
            (VIEWER_RECONNECT_GRACE..VIEWER_RECONNECT_GRACE + VIEWER_RECONNECT_POLL)
                .contains(&now.elapsed())
        );
        assert_eq!(viewer.status_code, StatusCode::CONFLICT);
        assert_eq!(viewer.message, ScreencastBusy::ProjectView.to_string());
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_admission_rechecks_the_fence_before_reserving() {
        let retired = Cell::new(false);
        let admission = reserve_after_viewer(|| {
            std::future::ready(if retired.get() {
                Err(stale_tab())
            } else {
                Ok(Err::<(), _>(ScreencastBusy::ProjectView))
            })
        });
        tokio::pin!(admission);
        assert!(futures::poll!(&mut admission).is_pending());
        retired.set(true);
        tokio::time::advance(VIEWER_RECONNECT_POLL).await;
        assert_eq!(admission.await.unwrap_err().status_code, StatusCode::GONE);
    }

    fn decode(bytes: &[u8]) -> (serde_json::Value, &[u8]) {
        let header_len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        let payload_len = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 8 + header_len + payload_len);
        let header = serde_json::from_slice(&bytes[8..8 + header_len]).unwrap();
        (header, &bytes[8 + header_len..])
    }

    #[test]
    fn control_records_have_bounded_utf8_headers_and_no_payload() {
        let heartbeat = encode(Record::Heartbeat, &[]).unwrap();
        assert_eq!(&heartbeat[..8], &[0, 0, 0, 20, 0, 0, 0, 0]);
        assert_eq!(
            decode(&heartbeat).0,
            serde_json::json!({"type": "heartbeat"})
        );
        let closed = encode_error(StreamFailure::Closed).unwrap();
        assert_eq!(
            decode(&closed).0,
            serde_json::json!({
                "type": "error", "status": 410, "detail": "stale tab_id: no such tab"
            })
        );
        assert!(encode(Record::Heartbeat, &[1]).is_none());
        assert!(encode_error(StreamFailure::Stopped).is_none());
    }

    #[test]
    fn header_payload_limits_are_on_wire_bytes_and_jpeg_is_verbatim() {
        let header = |url| Record::Frame {
            runtime_id: "r1",
            tab_id: "t1",
            mime: "image/jpeg",
            width: 1,
            height: 1,
            url,
            title: "Käse",
            viewport: Viewport {
                width: 2.0,
                height: 2.0,
                device_scale_factor: 2.0,
            },
        };
        let payload = vec![0xA5; MAX_JPEG_BYTES];
        let record = encode(header("about:blank"), &payload).unwrap();
        let (meta, encoded) = decode(&record);
        assert_eq!(meta["title"], "Käse");
        assert_eq!(meta["viewport"]["device_scale_factor"], 2.0);
        assert_eq!(encoded, payload);
        assert!(encode(header("about:blank"), &[]).is_none());
        assert!(encode(header("about:blank"), &vec![0; MAX_JPEG_BYTES + 1]).is_none());
        let escaped = "\\".repeat(MAX_HEADER_BYTES / 2);
        assert!(encode(header(&escaped), &[1]).is_none());
    }

    #[test]
    fn long_page_metadata_does_not_end_the_jpeg_stream_or_forge_a_url() {
        let frame = Frame {
            revision: 3,
            width: 4,
            height: 3,
            url: format!("https://example.test/{}", "x".repeat(MAX_HEADER_BYTES)),
            title: "🎞\u{0001}".repeat(MAX_FRAME_TITLE_BYTES),
            viewport: Viewport {
                width: 4.0,
                height: 3.0,
                device_scale_factor: 1.0,
            },
            jpeg: vec![0xff, 0xd8, 0xff, 0xd9],
        };
        let encoded = encode_frame(&frame, "rt", "t").unwrap();
        let (header, payload) = decode(&encoded);
        assert_eq!(header["type"], "frame");
        assert_eq!(header["url"], "");
        let title = header["title"].as_str().unwrap();
        assert!(title.len() <= MAX_FRAME_TITLE_BYTES && title.ends_with('…'));
        assert!(frame.title.starts_with(title.trim_end_matches('…')));
        assert_eq!(payload, frame.jpeg);
    }
}
