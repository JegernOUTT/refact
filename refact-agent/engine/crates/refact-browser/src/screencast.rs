use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;
use headless_chrome::protocol::cdp::types::Event;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImage, RgbaImage};
use serde::Serialize;

use refact_core::image_policy::{resize_to_policy, ImageFormat, ImagePolicy};

pub const DEFAULT_BURST_DURATION_MS: u64 = 1_000;
pub const MAX_BURST_DURATION_MS: u64 = 10_000;
pub const DEFAULT_FRAME_COUNT: usize = 8;
pub const MIN_FRAME_COUNT: usize = 2;
pub const MAX_FRAME_COUNT: usize = 24;
pub const MAX_SESSION_DURATION_MS: u64 = 30_000;
pub const MAX_SESSION_FRAMES: usize = 60;
pub const DEFAULT_SCREENCAST_QUALITY: u32 = 80;

const FILMSTRIP_MAX_COLUMNS: usize = 4;
const FILMSTRIP_MAX_ROWS: usize = 6;
const FILMSTRIP_PADDING: u32 = 8;
const FILMSTRIP_LABEL_HEIGHT: u32 = 16;
const FILMSTRIP_LABEL_SCALE: u32 = 2;
const CHANGE_THRESHOLD: f64 = 0.1;
const GLYPH_WIDTH: u32 = 5;
const GLYPH_HEIGHT: u32 = 7;
const SCREENCAST_POLL_MS: u64 = 25;
const SCREENCAST_PROBE_MS: u64 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBurstPlan {
    pub duration_ms: u64,
    pub frame_count: usize,
}

impl FrameBurstPlan {
    pub fn resolve(
        duration_ms: Option<u64>,
        frame_count: Option<usize>,
        interval_ms: Option<u64>,
    ) -> Result<Self, String> {
        if frame_count.is_some() && interval_ms.is_some() {
            return Err("capture_frames accepts frame_count or interval_ms, not both".to_string());
        }
        let duration_ms = duration_ms.unwrap_or(DEFAULT_BURST_DURATION_MS);
        if duration_ms == 0 {
            return Err("duration_ms must be greater than 0".to_string());
        }
        if duration_ms > MAX_BURST_DURATION_MS {
            return Err(format!(
                "duration_ms {duration_ms} exceeds the {MAX_BURST_DURATION_MS}ms capture cap"
            ));
        }
        let frame_count = match (frame_count, interval_ms) {
            (Some(frame_count), _) => frame_count,
            (None, Some(interval_ms)) => {
                if interval_ms == 0 {
                    return Err("interval_ms must be greater than 0".to_string());
                }
                (duration_ms / interval_ms) as usize + 1
            }
            (None, None) => DEFAULT_FRAME_COUNT,
        };
        if !(MIN_FRAME_COUNT..=MAX_FRAME_COUNT).contains(&frame_count) {
            let requested = match interval_ms {
                Some(interval_ms) => format!(
                    "interval_ms {interval_ms} over {duration_ms}ms yields {frame_count} frames"
                ),
                None => format!("frame_count {frame_count}"),
            };
            return Err(format!(
                "{requested}, outside the supported {MIN_FRAME_COUNT}..={MAX_FRAME_COUNT} range"
            ));
        }
        Ok(Self {
            duration_ms,
            frame_count,
        })
    }

    pub fn offsets(&self) -> Vec<u64> {
        let last = self.frame_count.saturating_sub(1).max(1) as u64;
        (0..self.frame_count)
            .map(|index| self.duration_ms * index as u64 / last)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub offset_ms: u64,
    pub data: Vec<u8>,
}

pub fn capture_timed_frames(
    plan: &FrameBurstPlan,
    mut capture: impl FnMut() -> Result<Vec<u8>, String>,
    mut wait_until: impl FnMut(Duration),
) -> Result<Vec<CapturedFrame>, String> {
    let started = Instant::now();
    let mut frames = Vec::with_capacity(plan.frame_count);
    for offset_ms in plan.offsets() {
        let target = started + Duration::from_millis(offset_ms);
        let remaining = target.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            wait_until(remaining);
        }
        let elapsed = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        frames.push(CapturedFrame {
            offset_ms: elapsed,
            data: capture()?,
        });
    }
    Ok(frames)
}

pub fn select_evenly_spaced(frames: Vec<CapturedFrame>, wanted: usize) -> Vec<CapturedFrame> {
    if frames.len() <= wanted || wanted == 0 {
        return frames;
    }
    let last = wanted.saturating_sub(1).max(1);
    let mut picked = Vec::with_capacity(wanted);
    let mut previous: Option<usize> = None;
    for slot in 0..wanted {
        let index = (frames.len() - 1) * slot / last;
        let index = match previous {
            Some(previous) if index <= previous => previous + 1,
            _ => index,
        };
        if index >= frames.len() {
            break;
        }
        previous = Some(index);
        picked.push(frames[index].clone());
    }
    picked
}

struct ScreencastFrameMessage {
    data: String,
    session_id: u32,
    received: Instant,
}

type ScreencastListener =
    std::sync::Weak<dyn headless_chrome::browser::tab::EventListener<Event> + Send + Sync>;

fn install_screencast_listener(
    tab: &Tab,
    sender: Sender<ScreencastFrameMessage>,
) -> Result<ScreencastListener, String> {
    tab.add_event_listener(Arc::new(move |event: &Event| {
        if let Event::PageScreencastFrame(frame) = event {
            let _ = sender.send(ScreencastFrameMessage {
                data: frame.params.data.clone(),
                session_id: frame.params.session_id,
                received: Instant::now(),
            });
        }
    }))
    .map_err(|error| format!("Failed to listen for browser screencast frames: {error}"))
}

fn start_screencast(
    tab: &Tab,
    quality: u32,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> Result<(), String> {
    tab.call_method(Page::StartScreencast {
        format: Some(Page::StartScreencastFormatOption::Jpeg),
        quality: Some(quality),
        max_width,
        max_height,
        every_nth_frame: Some(1),
    })
    .map(|_| ())
    .map_err(|error| format!("Failed to start browser screencast: {error}"))
}

fn stop_screencast(tab: &Tab) -> Result<(), String> {
    tab.call_method(Page::StopScreencast(None))
        .map(|_| ())
        .map_err(|error| format!("Failed to stop browser screencast: {error}"))
}

fn decode_frame(data: &str) -> Result<Vec<u8>, String> {
    base64::prelude::BASE64_STANDARD
        .decode(data)
        .map_err(|error| format!("Screencast frame decode failed: {error}"))
}

pub fn capture_screencast_burst(
    tab: &Tab,
    plan: &FrameBurstPlan,
    quality: u32,
) -> Result<Vec<CapturedFrame>, String> {
    let (sender, receiver) = mpsc::channel();
    let listener = install_screencast_listener(tab, sender)?;
    let started = Instant::now();
    let outcome = start_screencast(tab, quality, None, None).and_then(|_| {
        let deadline = started + Duration::from_millis(plan.duration_ms);
        let probe_deadline = started + Duration::from_millis(SCREENCAST_PROBE_MS);
        let mut frames = Vec::new();
        while Instant::now() < deadline && frames.len() < MAX_SESSION_FRAMES {
            if frames.is_empty() && Instant::now() >= probe_deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(remaining.min(Duration::from_millis(SCREENCAST_POLL_MS))) {
                Ok(message) => {
                    let _ = tab.call_method(Page::ScreencastFrameAck {
                        session_id: message.session_id,
                    });
                    frames.push(CapturedFrame {
                        offset_ms: message
                            .received
                            .saturating_duration_since(started)
                            .as_millis()
                            .min(u64::MAX as u128) as u64,
                        data: decode_frame(&message.data)?,
                    });
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(frames)
    });
    let _ = stop_screencast(tab);
    let _ = tab.remove_event_listener(&listener);
    outcome.map(|frames| select_evenly_spaced(frames, plan.frame_count))
}

pub struct ScreencastSessionOptions {
    pub quality: u32,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
}

impl Default for ScreencastSessionOptions {
    fn default() -> Self {
        Self {
            quality: DEFAULT_SCREENCAST_QUALITY,
            max_width: None,
            max_height: None,
        }
    }
}

struct ActiveScreencast {
    target_id: String,
    started: Instant,
    frames: Arc<Mutex<Vec<CapturedFrame>>>,
    auto_stopped: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    listener: ScreencastListener,
    drain: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreencastStopped {
    pub frames: Vec<CapturedFrame>,
    pub duration_ms: u64,
    pub auto_stopped: bool,
}

#[derive(Default)]
pub struct ScreencastManager {
    active: Arc<Mutex<Option<ActiveScreencast>>>,
}

impl ScreencastManager {
    pub fn start(&self, tab: &Arc<Tab>, options: ScreencastSessionOptions) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|error| format!("Failed to lock the browser screencast session: {error}"))?;
        if active.is_some() {
            return Err("A screencast session is already running".to_string());
        }
        let (sender, receiver) = mpsc::channel();
        let listener = install_screencast_listener(tab, sender)?;
        if let Err(error) =
            start_screencast(tab, options.quality, options.max_width, options.max_height)
        {
            let _ = tab.remove_event_listener(&listener);
            return Err(error);
        }
        let started = Instant::now();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let auto_stopped = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let drain = spawn_screencast_drain(
            tab.clone(),
            receiver,
            started,
            frames.clone(),
            auto_stopped.clone(),
            finished.clone(),
        );
        *active = Some(ActiveScreencast {
            target_id: tab.get_target_id().to_string(),
            started,
            frames,
            auto_stopped,
            finished,
            listener,
            drain: Some(drain),
        });
        Ok(())
    }

    pub fn stop(&self, tab: &Tab) -> Result<ScreencastStopped, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|error| format!("Failed to lock the browser screencast session: {error}"))?;
        match active.as_ref() {
            None => return Err("No screencast session is running".to_string()),
            Some(session) if session.target_id != *tab.get_target_id() => {
                return Err("The screencast session belongs to another tab".to_string());
            }
            Some(_) => {}
        }
        let mut session = active.take().unwrap_or_else(|| unreachable!());
        drop(active);
        Ok(self.finish(&mut session, tab))
    }

    pub fn is_running(&self) -> bool {
        self.active
            .lock()
            .map(|active| active.is_some())
            .unwrap_or(false)
    }

    pub fn cleanup(&self, tabs: &[Arc<Tab>]) {
        let session = self.active.lock().ok().and_then(|mut active| active.take());
        let Some(mut session) = session else {
            return;
        };
        let tab = tabs
            .iter()
            .find(|tab| tab.get_target_id().as_str() == session.target_id.as_str())
            .cloned();
        match tab {
            Some(tab) => {
                self.finish(&mut session, &tab);
            }
            None => {
                session.finished.store(true, Ordering::Relaxed);
                if let Some(drain) = session.drain.take() {
                    let _ = drain.join();
                }
            }
        }
    }

    fn finish(&self, session: &mut ActiveScreencast, tab: &Tab) -> ScreencastStopped {
        session.finished.store(true, Ordering::Relaxed);
        let _ = stop_screencast(tab);
        if let Some(drain) = session.drain.take() {
            let _ = drain.join();
        }
        let _ = tab.remove_event_listener(&session.listener);
        let frames = session
            .frames
            .lock()
            .map(|frames| frames.clone())
            .unwrap_or_default();
        ScreencastStopped {
            duration_ms: session.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            auto_stopped: session.auto_stopped.load(Ordering::Relaxed),
            frames,
        }
    }
}

fn spawn_screencast_drain(
    tab: Arc<Tab>,
    receiver: Receiver<ScreencastFrameMessage>,
    started: Instant,
    frames: Arc<Mutex<Vec<CapturedFrame>>>,
    auto_stopped: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let deadline = started + Duration::from_millis(MAX_SESSION_DURATION_MS);
        while !finished.load(Ordering::Relaxed) {
            match receiver.recv_timeout(Duration::from_millis(SCREENCAST_POLL_MS)) {
                Ok(message) => {
                    let _ = tab.call_method(Page::ScreencastFrameAck {
                        session_id: message.session_id,
                    });
                    let Ok(data) = decode_frame(&message.data) else {
                        continue;
                    };
                    let Ok(mut buffer) = frames.lock() else {
                        return;
                    };
                    if buffer.len() >= MAX_SESSION_FRAMES {
                        drop(buffer);
                        auto_stopped.store(true, Ordering::Relaxed);
                        let _ = stop_screencast(&tab);
                        return;
                    }
                    buffer.push(CapturedFrame {
                        offset_ms: message
                            .received
                            .saturating_duration_since(started)
                            .as_millis()
                            .min(u64::MAX as u128) as u64,
                        data,
                    });
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if Instant::now() >= deadline {
                auto_stopped.store(true, Ordering::Relaxed);
                let _ = stop_screencast(&tab);
                return;
            }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilmstripLayout {
    pub columns: usize,
    pub rows: usize,
    pub cell_width: u32,
    pub cell_height: u32,
}

impl FilmstripLayout {
    pub fn plan(
        frame_count: usize,
        frame_width: u32,
        frame_height: u32,
        max_side: u32,
    ) -> Result<Self, String> {
        if frame_count == 0 {
            return Err("A filmstrip needs at least one frame".to_string());
        }
        if frame_width == 0 || frame_height == 0 {
            return Err("Screencast frames have no pixels to compose".to_string());
        }
        let columns = frame_count.min(FILMSTRIP_MAX_COLUMNS);
        let rows = frame_count.div_ceil(columns);
        if rows > FILMSTRIP_MAX_ROWS {
            return Err(format!(
                "{frame_count} frames exceed the {FILMSTRIP_MAX_COLUMNS}x{FILMSTRIP_MAX_ROWS} filmstrip grid"
            ));
        }
        let scale = cell_scale(columns, rows, frame_width, frame_height, max_side)?;
        Ok(Self {
            columns,
            rows,
            cell_width: scaled_side(frame_width, scale),
            cell_height: scaled_side(frame_height, scale),
        })
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        (
            canvas_width(self.columns, self.cell_width),
            canvas_height(self.rows, self.cell_height),
        )
    }

    pub fn cell_origin(&self, index: usize) -> (u32, u32) {
        let column = (index % self.columns) as u32;
        let row = (index / self.columns) as u32;
        (
            FILMSTRIP_PADDING + column * (self.cell_width + FILMSTRIP_PADDING),
            FILMSTRIP_PADDING
                + row * (self.cell_height + FILMSTRIP_LABEL_HEIGHT + FILMSTRIP_PADDING),
        )
    }
}

fn canvas_width(columns: usize, cell_width: u32) -> u32 {
    FILMSTRIP_PADDING + columns as u32 * (cell_width + FILMSTRIP_PADDING)
}

fn canvas_height(rows: usize, cell_height: u32) -> u32 {
    FILMSTRIP_PADDING + rows as u32 * (cell_height + FILMSTRIP_LABEL_HEIGHT + FILMSTRIP_PADDING)
}

fn cell_scale(
    columns: usize,
    rows: usize,
    frame_width: u32,
    frame_height: u32,
    max_side: u32,
) -> Result<f64, String> {
    if max_side == 0 {
        return Ok(1.0);
    }
    let chrome_width = FILMSTRIP_PADDING * (columns as u32 + 1);
    let chrome_height =
        FILMSTRIP_PADDING * (rows as u32 + 1) + FILMSTRIP_LABEL_HEIGHT * rows as u32;
    let width_budget = max_side.saturating_sub(chrome_width);
    let height_budget = max_side.saturating_sub(chrome_height);
    if width_budget == 0 || height_budget == 0 {
        return Err(format!(
            "A {columns}x{rows} filmstrip grid does not fit inside a {max_side}px composite budget"
        ));
    }
    let width_scale = f64::from(width_budget) / (columns as f64 * f64::from(frame_width));
    let height_scale = f64::from(height_budget) / (rows as f64 * f64::from(frame_height));
    Ok(width_scale.min(height_scale).min(1.0))
}

fn scaled_side(side: u32, scale: f64) -> u32 {
    ((f64::from(side) * scale).floor() as u32).clamp(1, side.max(1))
}

pub fn frame_label(offset_ms: u64) -> String {
    format!("+{offset_ms}ms")
}

pub fn frame_change_percent(previous: &RgbaImage, current: &RgbaImage) -> Option<f64> {
    if previous.dimensions() != current.dimensions() {
        return None;
    }
    let total = u64::from(previous.width()) * u64::from(previous.height());
    if total == 0 {
        return Some(0.0);
    }
    let changed = previous
        .pixels()
        .zip(current.pixels())
        .filter(|(previous, current)| {
            previous
                .0
                .iter()
                .zip(current.0.iter())
                .map(|(left, right)| (f64::from(*left) - f64::from(*right)).abs() / 255.0)
                .fold(0.0, f64::max)
                > CHANGE_THRESHOLD
        })
        .count() as u64;
    Some(round_percent(changed as f64 * 100.0 / total as f64))
}

fn round_percent(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub fn compose_filmstrip(cells: &[(u64, RgbaImage)], layout: &FilmstripLayout) -> RgbaImage {
    let (width, height) = layout.canvas_size();
    let mut canvas = RgbaImage::from_pixel(width, height, image::Rgba([18, 18, 20, 255]));
    for (index, (offset_ms, cell)) in cells.iter().enumerate() {
        let (x, y) = layout.cell_origin(index);
        let _ = canvas.copy_from(cell, x, y);
        draw_label(
            &mut canvas,
            x,
            y + layout.cell_height + 2,
            &frame_label(*offset_ms),
            FILMSTRIP_LABEL_SCALE,
        );
    }
    canvas
}

fn glyph(character: char) -> Option<[u8; GLYPH_HEIGHT as usize]> {
    Some(match character {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        'm' => [0x00, 0x00, 0x1A, 0x15, 0x15, 0x15, 0x15],
        's' => [0x00, 0x00, 0x0F, 0x10, 0x0E, 0x01, 0x1E],
        _ => return None,
    })
}

fn draw_label(canvas: &mut RgbaImage, x: u32, y: u32, text: &str, scale: u32) {
    let scale = scale.max(1);
    let mut cursor = x;
    for character in text.chars() {
        let Some(rows) = glyph(character) else {
            cursor += (GLYPH_WIDTH + 1) * scale;
            continue;
        };
        for (row_index, row) in rows.iter().enumerate() {
            for column in 0..GLYPH_WIDTH {
                if row & (1 << (GLYPH_WIDTH - 1 - column)) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = cursor + column * scale + dx;
                        let py = y + row_index as u32 * scale + dy;
                        if px < canvas.width() && py < canvas.height() {
                            canvas.put_pixel(px, py, image::Rgba([245, 245, 245, 255]));
                        }
                    }
                }
            }
        }
        cursor += (GLYPH_WIDTH + 1) * scale;
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FrameArtifact {
    pub kind: &'static str,
    pub mime: String,
    pub path: PathBuf,
    pub bytes: usize,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FrameRecord {
    pub index: usize,
    pub offset_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_percent: Option<f64>,
    pub artifact: FrameArtifact,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FilmstripResult {
    pub frames: Vec<FrameRecord>,
    pub filmstrip: FrameArtifact,
    #[serde(skip)]
    pub filmstrip_data: String,
    pub columns: usize,
    pub rows: usize,
    pub duration_ms: u64,
    pub warnings: Vec<String>,
}

pub fn build_filmstrip(
    frames: &[CapturedFrame],
    artifacts_dir: &Path,
    label: &str,
    policy: &ImagePolicy,
    warnings: Vec<String>,
) -> Result<FilmstripResult, String> {
    if frames.is_empty() {
        return Err("The screencast produced no frames".to_string());
    }
    let decoded = frames
        .iter()
        .map(|frame| {
            image::load_from_memory(&frame.data)
                .map_err(|error| format!("Screencast frame decode failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let layout = FilmstripLayout::plan(
        decoded.len(),
        decoded[0].width(),
        decoded[0].height(),
        policy.preferred_side.min(policy.max_side),
    )?;
    let frame_policy = policy
        .clone()
        .with_format(ImageFormat::Jpeg, policy.quality);
    std::fs::create_dir_all(artifacts_dir).map_err(|error| {
        format!(
            "Failed to create browser artifacts directory {}: {error}",
            artifacts_dir.display()
        )
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut cells = Vec::with_capacity(decoded.len());
    let mut records = Vec::with_capacity(decoded.len());
    let mut previous_cell: Option<RgbaImage> = None;
    for (index, (frame, image)) in frames.iter().zip(decoded.iter()).enumerate() {
        let cell = image
            .resize_exact(layout.cell_width, layout.cell_height, FilterType::Triangle)
            .to_rgba8();
        let changed_percent = previous_cell
            .as_ref()
            .and_then(|previous| frame_change_percent(previous, &cell));
        let path = artifacts_dir.join(format!("frame-{label}-{nonce}-{index:02}.jpg"));
        let artifact = write_image_artifact("frame", &frame.data, &path, &frame_policy)?;
        records.push(FrameRecord {
            index,
            offset_ms: frame.offset_ms,
            changed_percent,
            artifact,
        });
        previous_cell = Some(cell.clone());
        cells.push((frame.offset_ms, cell));
    }

    let composed = compose_filmstrip(&cells, &layout);
    let mut encoded = Vec::new();
    DynamicImage::ImageRgba8(composed)
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .map_err(|error| format!("Filmstrip encode failed: {error}"))?;
    let path = artifacts_dir.join(format!("filmstrip-{label}-{nonce}.jpg"));
    let filmstrip = write_image_artifact("filmstrip", &encoded, &path, &frame_policy)?;
    let filmstrip_data = base64::prelude::BASE64_STANDARD.encode(
        std::fs::read(&filmstrip.path)
            .map_err(|error| format!("Failed to read the filmstrip artifact: {error}"))?,
    );

    Ok(FilmstripResult {
        duration_ms: frames
            .last()
            .map(|frame| frame.offset_ms)
            .unwrap_or_default(),
        frames: records,
        filmstrip,
        filmstrip_data,
        columns: layout.columns,
        rows: layout.rows,
        warnings,
    })
}

fn write_image_artifact(
    kind: &'static str,
    bytes: &[u8],
    path: &Path,
    policy: &ImagePolicy,
) -> Result<FrameArtifact, String> {
    let source_mime = image::guess_format(bytes)
        .map_err(|error| format!("Screencast frame format detection failed: {error}"))
        .map(|format| match format {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::WebP => "image/webp",
            _ => "image/jpeg",
        })?;
    let (encoded, mime) = resize_to_policy(bytes, source_mime, policy)?;
    let decoded = image::load_from_memory(&encoded)
        .map_err(|error| format!("Screencast frame decode failed: {error}"))?;
    std::fs::write(path, &encoded).map_err(|error| {
        format!(
            "Failed to save the screencast artifact {}: {error}",
            path.display()
        )
    })?;
    Ok(FrameArtifact {
        kind,
        mime,
        path: path.to_path_buf(),
        bytes: encoded.len(),
        width: decoded.width(),
        height: decoded.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_frame(offset_ms: u64, width: u32, height: u32, luma: u8) -> CapturedFrame {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([luma, luma, luma, 255]),
        ));
        let mut data = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut data),
                image::ImageFormat::Png,
            )
            .unwrap();
        CapturedFrame { offset_ms, data }
    }

    #[test]
    fn burst_plan_defaults_to_eight_frames_over_one_second() {
        let plan = FrameBurstPlan::resolve(None, None, None).unwrap();

        assert_eq!(
            plan,
            FrameBurstPlan {
                duration_ms: DEFAULT_BURST_DURATION_MS,
                frame_count: DEFAULT_FRAME_COUNT,
            }
        );
        assert_eq!(plan.offsets(), vec![0, 142, 285, 428, 571, 714, 857, 1_000]);
    }

    #[test]
    fn burst_plan_derives_frame_count_from_interval() {
        let plan = FrameBurstPlan::resolve(Some(1_000), None, Some(250)).unwrap();

        assert_eq!(plan.frame_count, 5);
        assert_eq!(plan.offsets(), vec![0, 250, 500, 750, 1_000]);
    }

    #[test]
    fn burst_plan_enforces_every_cap_with_a_clear_error() {
        assert_eq!(
            FrameBurstPlan::resolve(Some(10_001), None, None).unwrap_err(),
            "duration_ms 10001 exceeds the 10000ms capture cap"
        );
        assert_eq!(
            FrameBurstPlan::resolve(Some(1_000), Some(25), None).unwrap_err(),
            "frame_count 25, outside the supported 2..=24 range"
        );
        assert_eq!(
            FrameBurstPlan::resolve(Some(1_000), Some(1), None).unwrap_err(),
            "frame_count 1, outside the supported 2..=24 range"
        );
        assert_eq!(
            FrameBurstPlan::resolve(Some(1_000), None, Some(10)).unwrap_err(),
            "interval_ms 10 over 1000ms yields 101 frames, outside the supported 2..=24 range"
        );
        assert_eq!(
            FrameBurstPlan::resolve(Some(0), None, None).unwrap_err(),
            "duration_ms must be greater than 0"
        );
        assert_eq!(
            FrameBurstPlan::resolve(Some(1_000), None, Some(0)).unwrap_err(),
            "interval_ms must be greater than 0"
        );
        assert_eq!(
            FrameBurstPlan::resolve(Some(1_000), Some(4), Some(100)).unwrap_err(),
            "capture_frames accepts frame_count or interval_ms, not both"
        );
    }

    #[test]
    fn timed_capture_produces_one_frame_per_planned_offset() {
        let plan = FrameBurstPlan::resolve(Some(400), Some(5), None).unwrap();
        let mut captured = 0;
        let mut waited = Vec::new();

        let frames = capture_timed_frames(
            &plan,
            || {
                captured += 1;
                Ok(vec![captured as u8])
            },
            |remaining| waited.push(remaining),
        )
        .unwrap();

        assert_eq!(frames.len(), 5);
        assert_eq!(
            frames.iter().map(|frame| frame.data[0]).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert!(frames
            .windows(2)
            .all(|pair| pair[0].offset_ms <= pair[1].offset_ms));
        assert_eq!(waited.len(), 4);
    }

    #[test]
    fn frame_selection_spreads_across_the_burst_without_repeats() {
        let frames = (0..10)
            .map(|index| CapturedFrame {
                offset_ms: index * 10,
                data: vec![index as u8],
            })
            .collect::<Vec<_>>();

        let picked = select_evenly_spaced(frames.clone(), 4);

        assert_eq!(
            picked
                .iter()
                .map(|frame| frame.offset_ms)
                .collect::<Vec<_>>(),
            vec![0, 30, 60, 90]
        );
        assert_eq!(select_evenly_spaced(frames.clone(), 20).len(), 10);
        assert_eq!(
            select_evenly_spaced(frames, 3)
                .iter()
                .map(|frame| frame.offset_ms)
                .collect::<Vec<_>>(),
            vec![0, 40, 90]
        );
    }

    #[test]
    fn grid_layout_fills_four_columns_and_caps_at_six_rows() {
        let layout = FilmstripLayout::plan(8, 100, 50, 4_096).unwrap();
        assert_eq!(layout.columns, 4);
        assert_eq!(layout.rows, 2);
        assert_eq!((layout.cell_width, layout.cell_height), (100, 50));
        assert_eq!(layout.canvas_size(), (440, 156));
        assert_eq!(layout.cell_origin(0), (8, 8));
        assert_eq!(layout.cell_origin(4), (8, 82));
        assert_eq!(layout.cell_origin(5), (116, 82));

        assert_eq!(FilmstripLayout::plan(24, 10, 10, 4_096).unwrap().rows, 6);
        assert_eq!(FilmstripLayout::plan(3, 10, 10, 4_096).unwrap().columns, 3);
        assert!(FilmstripLayout::plan(25, 10, 10, 4_096).is_err());
        assert!(FilmstripLayout::plan(0, 10, 10, 4_096).is_err());
        assert!(FilmstripLayout::plan(4, 0, 10, 4_096).is_err());
    }

    #[test]
    fn grid_layout_downscales_cells_to_the_composite_pixel_budget() {
        let layout = FilmstripLayout::plan(8, 1_280, 800, 800).unwrap();

        assert_eq!((layout.cell_width, layout.cell_height), (190, 118));
        assert_eq!(layout.canvas_size(), (800, 292));
        assert!(FilmstripLayout::plan(24, 1_280, 800, 64).is_err());
    }

    #[test]
    fn timestamp_labels_render_inside_their_cell_strip() {
        let layout = FilmstripLayout::plan(2, 40, 40, 4_096).unwrap();
        let cells = vec![
            (
                0,
                RgbaImage::from_pixel(40, 40, image::Rgba([10, 10, 10, 255])),
            ),
            (
                125,
                RgbaImage::from_pixel(40, 40, image::Rgba([10, 10, 10, 255])),
            ),
        ];

        let canvas = compose_filmstrip(&cells, &layout);

        assert_eq!(frame_label(0), "+0ms");
        assert_eq!(frame_label(125), "+125ms");
        let (width, height) = layout.canvas_size();
        assert_eq!(canvas.dimensions(), (width, height));
        let label_row = FILMSTRIP_PADDING + layout.cell_height + 2;
        let label_pixels = (0..width)
            .flat_map(|x| {
                (label_row..label_row + GLYPH_HEIGHT * FILMSTRIP_LABEL_SCALE).map(move |y| (x, y))
            })
            .filter(|(x, y)| canvas.get_pixel(*x, *y).0 == [245, 245, 245, 255])
            .count();
        assert!(label_pixels > 0);
    }

    #[test]
    fn unsupported_label_characters_advance_without_drawing() {
        let mut canvas = RgbaImage::from_pixel(80, 20, image::Rgba([0, 0, 0, 255]));
        draw_label(&mut canvas, 0, 0, "?", 1);

        assert!(glyph('?').is_none());
        assert!(canvas.pixels().all(|pixel| pixel.0 == [0, 0, 0, 255]));
    }

    #[test]
    fn change_percent_measures_motion_between_consecutive_frames() {
        let still = RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 255]));
        let mut half = still.clone();
        for y in 0..5 {
            for x in 0..10 {
                half.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }

        assert_eq!(frame_change_percent(&still, &still), Some(0.0));
        assert_eq!(frame_change_percent(&still, &half), Some(50.0));
        assert_eq!(frame_change_percent(&still, &RgbaImage::new(4, 4)), None);
    }

    #[test]
    fn change_percent_ignores_deltas_below_the_visual_diff_threshold() {
        let base = RgbaImage::from_pixel(4, 4, image::Rgba([100, 100, 100, 255]));
        let nudged = RgbaImage::from_pixel(4, 4, image::Rgba([110, 110, 110, 255]));

        assert_eq!(frame_change_percent(&base, &nudged), Some(0.0));
    }

    #[test]
    fn filmstrip_build_writes_frame_and_filmstrip_artifacts_with_change_metrics() {
        let directory = tempfile::tempdir().unwrap();
        let artifacts = directory.path().join("artifacts");
        let frames = vec![
            solid_frame(0, 60, 40, 0),
            solid_frame(50, 60, 40, 0),
            solid_frame(100, 60, 40, 255),
        ];

        let result = build_filmstrip(
            &frames,
            &artifacts,
            "burst",
            &ImagePolicy::browser_capture(),
            vec!["captured with the timed fallback".to_string()],
        )
        .unwrap();

        assert_eq!(result.frames.len(), 3);
        assert_eq!(result.columns, 3);
        assert_eq!(result.rows, 1);
        assert_eq!(result.duration_ms, 100);
        assert_eq!(result.frames[0].changed_percent, None);
        assert_eq!(result.frames[1].changed_percent, Some(0.0));
        assert_eq!(result.frames[2].changed_percent, Some(100.0));
        assert_eq!(result.warnings, vec!["captured with the timed fallback"]);
        assert_eq!(result.filmstrip.kind, "filmstrip");
        assert_eq!(result.filmstrip.mime, "image/jpeg");
        assert!(result.filmstrip.path.exists());
        assert!(!result.filmstrip_data.is_empty());
        for record in &result.frames {
            assert_eq!(record.artifact.kind, "frame");
            assert!(record.artifact.path.exists());
            assert!(record.artifact.bytes > 0);
        }
    }

    #[test]
    fn filmstrip_build_rejects_an_empty_burst() {
        let directory = tempfile::tempdir().unwrap();

        assert_eq!(
            build_filmstrip(
                &[],
                directory.path(),
                "burst",
                &ImagePolicy::browser_capture(),
                Vec::new()
            )
            .unwrap_err(),
            "The screencast produced no frames"
        );
    }
}
