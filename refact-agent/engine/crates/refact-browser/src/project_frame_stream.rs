use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use headless_chrome::browser::transport::CommandDeadlineGuard;
use headless_chrome::protocol::cdp::{Page, types::Event};
use headless_chrome::Tab;
use serde::{Deserialize, Serialize};

use crate::screencast::{self, ScreencastBusy, ScreencastLease, ScreencastListener, ScreencastUse};
use crate::{RuntimeOwnership, WorldManager};

pub const MAX_JPEG_BYTES: usize = 4 * 1024 * 1024;
const MAX_BASE64_BYTES: usize = MAX_JPEG_BYTES.div_ceil(3) * 4;
const CDP_BUDGET: Duration = Duration::from_secs(1);
const METADATA_BUDGET: Duration = Duration::from_millis(200);
const POLL: Duration = Duration::from_millis(100);
const METADATA_REFRESH: Duration = Duration::from_secs(1);
const FIRST_FRAME_BUDGET: Duration = Duration::from_secs(6);
const RESUME_FRAME_BUDGET: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFailure {
    Replaced,
    Closed,
    Unavailable,
    ScopeChanged,
    Stopped,
}

pub struct ProjectFrameStream {
    tab: Arc<Tab>,
    epoch: u64,
    owner: Arc<RuntimeOwnership>,
    world: WorldManager,
    lease: ScreencastLease,
    event_revision: Arc<AtomicU64>,
}

pub struct StreamIdentity {
    tab: Arc<Tab>,
    epoch: u64,
    owner: Arc<RuntimeOwnership>,
    retired: Arc<AtomicBool>,
    event_revision: Arc<AtomicU64>,
}

impl StreamIdentity {
    pub fn check(&self) -> Result<(), StreamFailure> {
        let live = self.owner.has_stream_tab(&self.tab);
        if !self.owner.is_connection_epoch(self.epoch) {
            return Err(StreamFailure::Replaced);
        }
        if self.retired.load(Ordering::Acquire) || !live {
            return Err(StreamFailure::Closed);
        }
        if !self.owner.is_connected() {
            return Err(StreamFailure::Unavailable);
        }
        Ok(())
    }

    pub fn is_current_frame(&self, revision: u64) -> bool {
        self.event_revision.load(Ordering::Acquire) == revision
    }
}

#[derive(Serialize, Clone, Copy)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
    pub device_scale_factor: f64,
}

pub struct Frame {
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    pub url: String,
    pub title: String,
    pub viewport: Viewport,
    pub jpeg: Vec<u8>,
}

impl ProjectFrameStream {
    pub fn reserve(
        tab: Arc<Tab>,
        owner: Arc<RuntimeOwnership>,
        world: WorldManager,
    ) -> Result<Self, ScreencastBusy> {
        let lease = ScreencastLease::acquire(tab.get_target_id(), ScreencastUse::ProjectView)?;
        Ok(Self {
            tab,
            epoch: owner.connection_epoch(),
            owner,
            world,
            lease,
            event_revision: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn identity(&self) -> StreamIdentity {
        StreamIdentity {
            tab: self.tab.clone(),
            epoch: self.epoch,
            owner: self.owner.clone(),
            retired: self.lease.retired.clone(),
            event_revision: self.event_revision.clone(),
        }
    }

    pub fn run(
        self,
        quality: u32,
        max_side: u32,
        check: impl Fn() -> Result<(), StreamFailure>,
        mut maintain: impl FnMut(),
        mut publish: impl FnMut(Frame) -> Result<(), StreamFailure>,
    ) -> Result<(), StreamFailure> {
        check()?;
        let identity = self.identity();
        let mut started = start(&self.tab, self.event_revision.clone(), quality, max_side)?;
        let mut sample: Option<Metadata> = None;
        let mut last_timestamp = 0.0;
        let mut first_deadline = Some(Instant::now() + FIRST_FRAME_BUDGET);
        let mut revision_wait = (self.event_revision.load(Ordering::Acquire), None);
        let mut primed_revision = None;
        let mut rejected_since: Option<Instant> = None;
        loop {
            check()?;
            maintain();
            let revision = self.event_revision.load(Ordering::Acquire);
            if revision != revision_wait.0 {
                revision_wait = (revision, Some(Instant::now() + RESUME_FRAME_BUDGET));
            }
            if revision_wait
                .1
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                // A static document may not draw again after the only candidate was rejected
                // during navigation/resize. Re-prime the compositor once for that revision.
                if primed_revision == Some(revision) {
                    return Err(StreamFailure::Unavailable);
                }
                primed_revision = Some(revision);
                drop(started);
                check()?;
                started = start(&self.tab, self.event_revision.clone(), quality, max_side)?;
                revision_wait.1 = Some(Instant::now() + FIRST_FRAME_BUDGET);
            }
            if started.overflow.load(Ordering::Acquire)
                || first_deadline.is_some_and(|deadline| Instant::now() >= deadline)
                || rejected_since.is_some_and(|since| since.elapsed() >= FIRST_FRAME_BUDGET)
            {
                return Err(StreamFailure::Unavailable);
            }
            let raw = match started.frames.recv_timeout(POLL) {
                Ok(raw) => raw,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err(StreamFailure::Unavailable),
            };
            if within(CDP_BUDGET, || self.tab.ack_screencast(raw.session_id)).is_err() {
                check()?;
                return Err(StreamFailure::Unavailable);
            }
            rejected_since.get_or_insert_with(Instant::now);
            if !identity.is_current_frame(raw.revision)
                || raw
                    .geometry
                    .timestamp
                    .is_some_and(|time| time < raw.valid_after || time < last_timestamp)
            {
                continue;
            }
            if let Some(time) = raw.geometry.timestamp {
                last_timestamp = time;
            }
            if sample
                .as_ref()
                .is_none_or(|cached| cached.needs_refresh(&raw))
            {
                match Metadata::read(&self.tab, &self.world, &raw) {
                    Some(next) => sample = Some(next),
                    None if sample.as_ref().is_some_and(|cached| cached.matches(&raw)) => {
                        // A slow renderer must not turn a known stationary geometry into
                        // a metadata RPC on every frame.
                        if let Some(cached) = &mut sample {
                            cached.sampled_at = Instant::now();
                        }
                    }
                    None => continue,
                }
            }
            let Some(sample) = sample.as_ref() else {
                continue;
            };
            if !identity.is_current_frame(raw.revision) {
                continue;
            }
            let jpeg = base64::prelude::BASE64_STANDARD
                .decode(raw.data)
                .ok()
                .filter(|data| data.len() <= MAX_JPEG_BYTES)
                .ok_or(StreamFailure::Unavailable)?;
            let (width, height) = jpeg_dimensions(&jpeg).ok_or(StreamFailure::Unavailable)?;
            // Chromium may finish encoding an old surface after its geometry changed.
            // Do not pair it with the new viewport, even if it arrived last.
            if !raw.geometry.matches_image(width, height) {
                continue;
            }
            if width.max(height) > max_side {
                return Err(StreamFailure::Unavailable);
            }
            let frame = Frame {
                revision: raw.revision,
                width,
                height,
                url: sample.url.clone(),
                title: sample.title.clone(),
                viewport: raw
                    .geometry
                    .css_viewport(sample)
                    .ok_or(StreamFailure::Unavailable)?,
                jpeg,
            };
            check()?;
            if identity.is_current_frame(frame.revision) {
                publish(frame)?;
                first_deadline = None;
                revision_wait = (raw.revision, None);
                rejected_since = None;
            }
        }
    }
}

struct RawFrame {
    frame_id: Arc<str>,
    revision: u64,
    valid_after: f64,
    session_id: u32,
    data: String,
    geometry: Geometry,
}

#[derive(Clone, Copy)]
struct Geometry {
    width: f64,
    height: f64,
    offset_top: f64,
    page_scale: f64,
    timestamp: Option<f64>,
}

impl Geometry {
    fn matches_image(&self, width: u32, height: u32) -> bool {
        let height_dip = self.height - self.offset_top;
        if self.width <= 0.0 || height_dip <= 0.0 {
            return false;
        }
        let scale = (f64::from(width) / self.width).min(f64::from(height) / height_dip);
        // Account for capture dimensions rounded in Chrome, and for integral DIP metadata.
        (self.width * scale - f64::from(width)).abs() <= 3.0
            && (height_dip * scale - f64::from(height)).abs() <= 3.0
    }

    fn css_viewport(&self, metadata: &Metadata) -> Option<Viewport> {
        let scale = metadata.zoom * self.page_scale;
        let width = self.width / scale;
        let height = (self.height - self.offset_top) / scale;
        (width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0).then_some(
            Viewport {
                width,
                height,
                device_scale_factor: metadata.dpr,
            },
        )
    }

    fn same_surface(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.offset_top == other.offset_top
            && self.page_scale == other.page_scale
    }
}

struct Metadata {
    revision: u64,
    sampled_at: Instant,
    surface: Geometry,
    url: String,
    title: String,
    dpr: f64,
    zoom: f64,
}

const METADATA_EXPRESSION: &str = r"(() => {
    if (window !== window.top) return null;
    const root = document.compatMode === 'BackCompat' ? document.body : document.documentElement;
    const view = window.visualViewport;
    if (!view) return null;
    const dpr = devicePixelRatio;
    const view_width = view.width;
    const view_height = view.height;
    const view_scale = view.scale;
    const gutter_x = root ? innerWidth - root.clientWidth : 0;
    const gutter_y = root ? innerHeight - root.clientHeight : 0;
    const origin = performance.timeOrigin / 1000;
    if (![dpr, view_width, view_height, view_scale, gutter_x, gutter_y, origin]
        .every(Number.isFinite)) return null;
    const href = location.href;
    const title = document.title;
    if (typeof href !== 'string' || typeof title !== 'string') return null;
    return {
        url: href.length <= 8192 ? href : '',
        title: (title.length <= 512 ? title : title.slice(0, 512) + '…')
            .replace(/[\uD800-\uDFFF]/gu, '\uFFFD'),
        dpr, view_width, view_height, view_scale, gutter_x, gutter_y, origin,
    };
})()";

#[derive(Deserialize)]
struct DocumentMetadata {
    url: String,
    title: String,
    dpr: f64,
    view_width: f64,
    view_height: f64,
    view_scale: f64,
    gutter_x: f64,
    gutter_y: f64,
    origin: f64,
}

impl Metadata {
    fn read(tab: &Tab, world: &WorldManager, raw: &RawFrame) -> Option<Self> {
        let DocumentMetadata {
            url,
            title,
            dpr,
            view_width,
            view_height,
            view_scale,
            gutter_x,
            gutter_y,
            origin,
        } = within(METADATA_BUDGET, || {
            let value =
                world.eval_project_stream_metadata(tab, &raw.frame_id, METADATA_EXPRESSION)?;
            serde_json::from_value(value).ok()
        })?;
        if !dpr.is_finite() || dpr <= 0.0 || !view_scale.is_finite() || view_scale <= 0.0 {
            return None;
        }
        if raw
            .geometry
            .timestamp
            .is_some_and(|timestamp| timestamp < origin)
        {
            return None;
        }
        let metrics = within(METADATA_BUDGET, || {
            tab.call_method(Page::GetLayoutMetrics(None))
        })
        .ok()?;
        let zoom = metrics.css_visual_viewport.zoom.unwrap_or(1.0);
        let full_width = (view_width * view_scale + gutter_x.max(0.0)) * zoom;
        let full_height = (view_height * view_scale + gutter_y.max(0.0)) * zoom;
        if (raw.geometry.width - full_width).abs() > 2.0
            || (raw.geometry.height - raw.geometry.offset_top - full_height).abs() > 2.0
            || (view_scale - raw.geometry.page_scale).abs() > 0.01
        {
            return None;
        }
        Some(Self {
            revision: raw.revision,
            sampled_at: Instant::now(),
            surface: raw.geometry,
            url,
            title,
            dpr,
            zoom,
        })
    }

    fn matches(&self, raw: &RawFrame) -> bool {
        self.revision == raw.revision && self.surface.same_surface(&raw.geometry)
    }

    fn needs_refresh(&self, raw: &RawFrame) -> bool {
        !self.matches(raw) || self.sampled_at.elapsed() >= METADATA_REFRESH
    }
}

fn within<T>(duration: Duration, call: impl FnOnce() -> T) -> T {
    let _deadline = CommandDeadlineGuard::new(Instant::now() + duration);
    call()
}

fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

struct Started<'a> {
    tab: &'a Tab,
    listener: ScreencastListener,
    frames: Receiver<RawFrame>,
    overflow: Arc<AtomicBool>,
}

impl Drop for Started<'_> {
    fn drop(&mut self) {
        let _ = self.tab.remove_event_listener(&self.listener);
        let _ = within(CDP_BUDGET, || screencast::stop_screencast(self.tab));
    }
}

fn start(
    tab: &Tab,
    event_revision: Arc<AtomicU64>,
    quality: u32,
    max_side: u32,
) -> Result<Started<'_>, StreamFailure> {
    let tree = within(CDP_BUDGET, || tab.call_method(Page::GetFrameTree(None)))
        .map_err(|_| StreamFailure::Unavailable)?;
    let page_state = Mutex::new((Arc::<str>::from(tree.frame_tree.frame.id), 0.0));
    let (sender, frames) = mpsc::sync_channel(4);
    let overflow = Arc::new(AtomicBool::new(false));
    let listener = tab
        .add_event_listener(Arc::new({
            let overflow = overflow.clone();
            move |event: &Event| {
                let mut state = page_state.lock().unwrap();
                match event {
                    Event::PageFrameNavigated(event) if event.params.frame.parent_id.is_none() => {
                        *state = (Arc::from(event.params.frame.id.as_str()), unix_seconds());
                        event_revision.fetch_add(1, Ordering::AcqRel);
                    }
                    Event::PageNavigatedWithinDocument(event)
                        if event.params.frame_id.as_str() == state.0.as_ref() =>
                    {
                        state.1 = unix_seconds();
                        event_revision.fetch_add(1, Ordering::AcqRel);
                    }
                    Event::PageFrameResized(_) => {
                        event_revision.fetch_add(1, Ordering::AcqRel);
                    }
                    Event::PageScreencastFrame(event) => {
                        let params = &event.params;
                        let meta = &params.metadata;
                        let data = (params.data.len() <= MAX_BASE64_BYTES)
                            .then(|| params.data.clone())
                            .unwrap_or_default();
                        let frame = RawFrame {
                            data,
                            frame_id: state.0.clone(),
                            revision: event_revision.load(Ordering::Acquire),
                            valid_after: state.1,
                            session_id: params.session_id,
                            geometry: Geometry {
                                width: meta.device_width,
                                height: meta.device_height,
                                offset_top: meta.offset_top,
                                page_scale: meta.page_scale_factor,
                                timestamp: meta.timestamp,
                            },
                        };
                        if let Err(TrySendError::Full(_)) = sender.try_send(frame) {
                            overflow.store(true, Ordering::Release);
                        }
                    }
                    _ => {}
                }
            }
        }))
        .map_err(|_| StreamFailure::Unavailable)?;
    let started = Started {
        tab,
        listener,
        frames,
        overflow,
    };
    within(CDP_BUDGET, || {
        screencast::start_screencast(tab, quality, Some(max_side), Some(max_side))
    })
    .map_err(|_| StreamFailure::Unavailable)?;
    Ok(started)
}

fn jpeg_dimensions(mut data: &[u8]) -> Option<(u32, u32)> {
    data = data.strip_prefix(&[0xff, 0xd8])?;
    while let Some(rest) = data.strip_prefix(&[0xff]) {
        data = rest;
        while let Some(rest) = data.strip_prefix(&[0xff]) {
            data = rest;
        }
        let (&marker, rest) = data.split_first()?;
        if matches!(marker, 0x00 | 0xd8 | 0xd9 | 0xda) {
            return None;
        }
        let len = u16::from_be_bytes(rest.get(..2)?.try_into().ok()?) as usize;
        if len < 2 {
            return None;
        }
        let segment = rest.get(..len)?;
        if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let height = u32::from(u16::from_be_bytes(segment.get(3..5)?.try_into().ok()?));
            let width = u32::from(u16::from_be_bytes(segment.get(5..7)?.try_into().ok()?));
            return (width != 0 && height != 0).then_some((width, height));
        }
        data = &rest[len..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_dimensions_are_read_without_touching_pixel_data() {
        let encoded = [
            0xff, 0xd8, 0xff, 0xe0, 0, 5, 11, 12, 13, 0xff, 0xc2, 0, 8, 8, 0x01, 0x35, 0x04, 0xd2,
            3,
        ];
        assert_eq!(jpeg_dimensions(&encoded), Some((1234, 309)));
        assert_eq!(jpeg_dimensions(&encoded[..18]), None);
        assert_eq!(jpeg_dimensions(&[0xff, 0xd8, 0xff, 0xda, 0, 2]), None);
        assert_eq!(jpeg_dimensions(&[0xff, 0xd8, 0xff, 0xe0, 0, 1]), None);
    }

    #[test]
    fn event_surface_maps_jpeg_pixels_to_css_without_using_jpeg_scale_as_dpr() {
        let geometry = Geometry {
            width: 400.0,
            height: 800.0,
            offset_top: 0.0,
            page_scale: 0.5,
            timestamp: Some(15.0),
        };
        let sample = Metadata {
            revision: 2,
            sampled_at: Instant::now(),
            surface: geometry,
            url: "about:blank".into(),
            title: "test".into(),
            dpr: 3.0,
            zoom: 1.0,
        };
        assert!(geometry.matches_image(200, 400));
        assert!(!geometry.matches_image(400, 200));
        let viewport = geometry.css_viewport(&sample).unwrap();
        assert_eq!(
            (
                viewport.width,
                viewport.height,
                viewport.device_scale_factor
            ),
            (800.0, 1600.0, 3.0)
        );
    }
}
