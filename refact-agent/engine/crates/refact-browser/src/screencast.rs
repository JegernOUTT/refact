use std::path::PathBuf;
use crate::artifact_store::ArtifactStore;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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

pub const TIMED_CAPTURE_MIN_OVERRUN_MS: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TimedCapture {
    pub frames: Vec<CapturedFrame>,
    pub planned: usize,
    pub dropped_frames: usize,
    pub budget_ms: u64,
    pub elapsed_ms: u64,
    pub budget_exhausted: bool,
}

impl TimedCapture {
    pub fn warning(&self) -> Option<String> {
        if !self.budget_exhausted && self.dropped_frames == 0 && self.frames.len() >= 2 {
            return None;
        }
        let mut warning = format!(
            "capture_frames recorded {} of {} planned frames in {}ms ({}ms wall-clock budget for scheduling; synchronous captures may overrun it)",
            self.frames.len(),
            self.planned,
            self.elapsed_ms,
            self.budget_ms
        );
        if self.budget_exhausted {
            warning.push_str("; scheduling budget exhausted");
        }
        if self.frames.len() < 2 {
            warning.push_str("; fewer than two frames are insufficient evidence of motion");
        }
        Some(warning)
    }
}

/// Scheduling budget for a timed burst. Synchronous captures can exceed this budget.
pub fn timed_capture_budget(plan: &FrameBurstPlan) -> Duration {
    Duration::from_millis(plan.duration_ms + plan.duration_ms.max(TIMED_CAPTURE_MIN_OVERRUN_MS))
}

pub fn capture_timed_frames(
    plan: &FrameBurstPlan,
    capture: impl FnMut() -> Result<Vec<u8>, String>,
    wait_until: impl FnMut(Duration),
) -> Result<TimedCapture, String> {
    capture_timed_frames_within(
        plan,
        timed_capture_budget(plan),
        capture_now,
        capture,
        wait_until,
    )
}

fn capture_now() -> Instant {
    Instant::now()
}

/// Schedule frames against one absolute budget, checking both before and after each wait.
/// This cannot interrupt a synchronous capture already in progress. Frame offsets are
/// measured at capture start; elapsed time includes capture work.
pub fn capture_timed_frames_within(
    plan: &FrameBurstPlan,
    budget: Duration,
    mut now: impl FnMut() -> Instant,
    mut capture: impl FnMut() -> Result<Vec<u8>, String>,
    mut wait_until: impl FnMut(Duration),
) -> Result<TimedCapture, String> {
    let started = now();
    let offsets = plan.offsets();
    let planned = offsets.len();
    let mut frames = Vec::with_capacity(planned);
    let mut budget_exhausted = false;
    for offset_ms in offsets {
        let target = started + Duration::from_millis(offset_ms);
        let current = now();
        if current.saturating_duration_since(started) >= budget {
            budget_exhausted = true;
            break;
        }
        let remaining = target
            .saturating_duration_since(current)
            .min(budget.saturating_sub(current.saturating_duration_since(started)));
        if !remaining.is_zero() {
            wait_until(remaining);
        }
        let elapsed_duration = now().saturating_duration_since(started);
        if elapsed_duration >= budget {
            budget_exhausted = true;
            break;
        }
        let elapsed = elapsed_duration.as_millis().min(u64::MAX as u128) as u64;
        frames.push(CapturedFrame {
            offset_ms: elapsed,
            data: capture()?,
        });
    }
    let elapsed = now().saturating_duration_since(started);
    budget_exhausted |= elapsed >= budget;
    let elapsed_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
    Ok(TimedCapture {
        dropped_frames: planned.saturating_sub(frames.len()),
        frames,
        planned,
        budget_ms: budget.as_millis().min(u64::MAX as u128) as u64,
        elapsed_ms,
        budget_exhausted,
    })
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
    pub fn has_active_session(&self) -> bool {
        self.active
            .lock()
            .map(|session| session.is_some())
            .unwrap_or(true)
    }

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
    store: &ArtifactStore,
    _label: &str,
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
        let artifact = write_image_artifact("frame", &frame.data, store, &frame_policy)?.0;
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
    let (filmstrip, filmstrip_bytes) =
        write_image_artifact("filmstrip", &encoded, store, &frame_policy)?;
    let filmstrip_data = base64::prelude::BASE64_STANDARD.encode(filmstrip_bytes);

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
    store: &ArtifactStore,
    policy: &ImagePolicy,
) -> Result<(FrameArtifact, Vec<u8>), String> {
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
    let stored = store.write(&encoded, &mime)?;
    Ok((
        FrameArtifact {
            kind,
            mime,
            path: stored.path,
            bytes: encoded.len(),
            width: decoded.width(),
            height: decoded.height(),
        },
        encoded,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filmstrip_artifacts_use_shared_store_and_keep_encoded_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path()).unwrap();
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 6, image::Rgba([255, 0, 0, 255])));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let frames = vec![CapturedFrame {
            offset_ms: 0,
            data: bytes.into_inner(),
        }];
        let result =
            build_filmstrip(&frames, &store, "test", &ImagePolicy::default(), vec![]).unwrap();
        let saved = std::fs::read(&result.frames[0].artifact.path).unwrap();
        let decoded = image::load_from_memory(&saved).unwrap();
        assert_eq!(result.frames[0].artifact.bytes, saved.len());
        assert_eq!(
            (
                result.frames[0].artifact.width,
                result.frames[0].artifact.height
            ),
            (decoded.width(), decoded.height())
        );
        assert_eq!(
            base64::prelude::BASE64_STANDARD
                .decode(&result.filmstrip_data)
                .unwrap(),
            std::fs::read(&result.filmstrip.path).unwrap()
        );
        let first = result.filmstrip.path;
        for _ in 0..64 {
            store.write(b"other", "text/plain").unwrap();
        }
        assert!(!first.exists(), "filmstrip bypassed shared quota");
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

    struct FakeClock {
        now: std::cell::Cell<Instant>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                now: std::cell::Cell::new(Instant::now()),
            }
        }

        fn advance(&self, duration: Duration) {
            self.now.set(self.now.get() + duration);
        }
    }

    #[test]
    fn a_burst_stops_at_the_budget_and_reports_the_dropped_frames() {
        let plan = FrameBurstPlan::resolve(Some(1_000), Some(8), None).unwrap();
        let clock = FakeClock::new();
        let mut captured = 0u8;

        let capture = capture_timed_frames_within(
            &plan,
            timed_capture_budget(&plan),
            || clock.now.get(),
            || {
                captured += 1;
                clock.advance(Duration::from_millis(1_500));
                Ok(vec![captured])
            },
            |remaining| clock.advance(remaining),
        )
        .unwrap();

        assert_eq!(capture.frames.len(), 4);
        assert_eq!(capture.dropped_frames, 4);
        assert!(capture.budget_exhausted);
        assert_eq!(capture.budget_ms, 6_000);
        let warning = capture.warning().unwrap();
        assert!(warning.contains("4 of 8 planned frames"), "{warning}");
        assert!(warning.contains("6000ms wall-clock budget"), "{warning}");
    }

    #[test]
    fn a_capture_path_slower_than_the_budget_stops_after_the_first_frame() {
        let plan = FrameBurstPlan::resolve(Some(300), Some(3), None).unwrap();
        let clock = FakeClock::new();
        let mut captured = 0u8;

        let capture = capture_timed_frames_within(
            &plan,
            timed_capture_budget(&plan),
            || clock.now.get(),
            || {
                captured += 1;
                clock.advance(Duration::from_millis(15_000));
                Ok(vec![captured])
            },
            |remaining| clock.advance(remaining),
        )
        .unwrap();

        assert_eq!(capture.frames.len(), 1);
        assert_eq!(capture.budget_ms, 5_300);
        assert!(capture.budget_exhausted);
        assert_eq!(capture.elapsed_ms, 15_000);
        assert_eq!(capture.dropped_frames, 2);
        let warning = capture.warning().unwrap();
        assert!(warning.contains("1 of 3 planned frames"), "{warning}");
        assert!(warning.contains("5300ms wall-clock budget"), "{warning}");
        assert!(
            warning.contains("insufficient evidence of motion"),
            "{warning}"
        );
    }

    #[test]
    fn exhausted_budget_never_starts_even_the_first_capture() {
        let plan = FrameBurstPlan::resolve(Some(800), Some(5), None).unwrap();
        let clock = FakeClock::new();
        let capture = capture_timed_frames_within(
            &plan,
            Duration::ZERO,
            || clock.now.get(),
            || panic!("capture must not start after deadline"),
            |_| panic!("wait must not start after deadline"),
        )
        .unwrap();
        assert_eq!(capture.frames.len(), 0);
        assert_eq!(capture.dropped_frames, 5);
        assert!(capture.budget_exhausted);
        assert!(
            capture
                .warning()
                .unwrap()
                .contains("insufficient evidence of motion")
        );
    }

    #[test]
    fn oversleep_does_not_start_another_capture() {
        let plan = FrameBurstPlan::resolve(Some(800), Some(5), None).unwrap();
        let clock = FakeClock::new();
        let mut calls = 0;
        let capture = capture_timed_frames_within(
            &plan,
            Duration::from_millis(300),
            || clock.now.get(),
            || {
                calls += 1;
                Ok(vec![1])
            },
            |_| clock.advance(Duration::from_millis(400)),
        )
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(capture.elapsed_ms, 400);
        assert_eq!(capture.dropped_frames, 4);
        assert!(capture.budget_exhausted);
    }

    #[test]
    fn final_capture_overrun_is_reported_even_without_dropped_frames() {
        let plan = FrameBurstPlan::resolve(Some(100), Some(2), None).unwrap();
        let clock = FakeClock::new();
        let capture = capture_timed_frames_within(
            &plan,
            Duration::from_millis(200),
            || clock.now.get(),
            || {
                clock.advance(Duration::from_millis(150));
                Ok(vec![1])
            },
            |duration| clock.advance(duration),
        )
        .unwrap();
        assert_eq!(capture.frames.len(), 2);
        assert_eq!(capture.elapsed_ms, 300);
        assert!(capture.budget_exhausted);
        assert!(
            capture
                .warning()
                .unwrap()
                .contains("synchronous captures may overrun")
        );
    }
}
