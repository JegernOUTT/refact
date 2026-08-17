use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use headless_chrome::protocol::cdp::{Emulation, Input};
use headless_chrome::Tab;
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use refact_browser::{
    MediaState, SnapshotBox, SnapshotNode, SnapshotOptions, ViewportState, WorldManager,
};
use refact_core::image_policy::{resize_to_policy, ImagePolicy};
use refact_integrations::browser_models::{
    BrowserLocator, BrowserScreenshotAnimations, BrowserScreenshotCaret, BrowserScreenshotOptions,
    BrowserScreenshotType, BrowserStep,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::codegraph::code_intel_api::ToolJson;
use crate::integrations::browser_controller;
use crate::integrations::browser_runtime::{find_runtime_by_chat_id, BrowserRuntime};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const DEFAULT_VIEWPORT_HEIGHT: u32 = 900;
const DEFAULT_DIFF_THRESHOLD: f64 = 0.1;
const MAX_TARGETS: usize = 50;
const MAX_MATRIX_CELLS: usize = 400;
const MAX_AUDIT_FINDINGS: usize = 500;
const DEFAULT_DESIGN_TOKEN_STYLES: &[&str] = &["src/styles/tokens.css"];
const PAGE_NOT_INSTRUMENTED_ERROR: &str =
    "page not instrumented for design tools (requires the local Vite dev-server flow)";
const INSTRUMENTATION_ERROR_MARKERS: &[&str] = &[
    "Unknown RefactInjected method",
    "RefactInjected is not installed",
    "__refact_injected__ is not defined",
];
const ZERO_SCAN_WARNING: &str = "Contrast audit scanned 0 elements — page not instrumented or selector matched nothing; results are NOT a pass";
const NO_TOKEN_FILES_WARNING: &str = "Contrast audit resolved 0 design-token files — non-token color findings are incomplete; results are NOT a pass";
const PROBE_FUNCTION: &str = r#"function(properties) {
  const el = this;
  const rect = el.getBoundingClientRect();
  const style = getComputedStyle(el);
  const styles = {};
  for (const name of properties) styles[name] = style.getPropertyValue(name);
  return {
    box: {x: rect.x, y: rect.y, width: rect.width, height: rect.height},
    styles,
    overflow: {
      x: el.scrollWidth > el.clientWidth,
      y: el.scrollHeight > el.clientHeight,
      viewport: rect.left < 0 || rect.top < 0 || rect.right > innerWidth || rect.bottom > innerHeight
    }
  };
}"#;
const CONTRAST_AUDIT_EXPRESSION: &str = r#"(() => {
  const selectorFor = (el) => {
    if (el.id) return '#' + CSS.escape(el.id);
    const parts = [];
    let current = el;
    while (current && current.nodeType === Node.ELEMENT_NODE && parts.length < 6) {
      let part = current.localName;
      const siblings = current.parentElement ? Array.from(current.parentElement.children).filter(s => s.localName === current.localName) : [];
      if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(current) + 1) + ')';
      parts.unshift(part);
      current = current.parentElement;
    }
    return parts.join(' > ');
  };
  const result = [];
  const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    const text = (node.textContent || '').trim();
    const el = node.parentElement;
    if (!text || !el) continue;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    if (!rect.width || !rect.height || style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) continue;
    let background = 'rgba(0, 0, 0, 0)';
    let cursor = el;
    while (cursor) {
      const candidate = getComputedStyle(cursor).backgroundColor;
      if (candidate && candidate !== 'transparent' && !candidate.endsWith(', 0)')) { background = candidate; break; }
      cursor = cursor.parentElement;
    }
    if (background === 'rgba(0, 0, 0, 0)') background = getComputedStyle(document.documentElement).backgroundColor || 'rgb(255, 255, 255)';
    result.push({kind:'text', selector: selectorFor(el), text: text.slice(0, 120), foreground: style.color, background, fontSize: parseFloat(style.fontSize) || 16, fontWeight: style.fontWeight});
  }
  for (const el of document.querySelectorAll('button,input,select,textarea,[role=button],[role=checkbox],[role=radio],[role=switch]')) {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    if (!rect.width || !rect.height || style.display === 'none' || style.visibility === 'hidden') continue;
    result.push({kind:'non_text', selector: selectorFor(el), text: '', foreground: style.borderColor || style.color, background: style.backgroundColor, fontSize: 0, fontWeight: '400'});
  }
  return result;
})()"#;

const DEFAULT_STYLE_PROPERTIES: &[&str] = &[
    "display",
    "position",
    "color",
    "background-color",
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "letter-spacing",
    "margin",
    "padding",
    "border",
    "border-radius",
    "box-shadow",
    "opacity",
    "overflow-x",
    "overflow-y",
    "z-index",
];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum TargetInput {
    Text(String),
    Structured {
        #[serde(rename = "ref")]
        reference: Option<String>,
        selector: Option<String>,
    },
}

impl TargetInput {
    fn locator(&self) -> Result<BrowserLocator, String> {
        match self {
            Self::Text(value) if value.starts_with('e') || value.starts_with('f') => {
                Ok(BrowserLocator::reference(value))
            }
            Self::Text(value) if !value.trim().is_empty() => Ok(BrowserLocator::css(value)),
            Self::Text(_) => Err("target must not be empty".to_string()),
            Self::Structured {
                reference: Some(reference),
                selector: None,
            } => Ok(BrowserLocator::reference(reference)),
            Self::Structured {
                reference: None,
                selector: Some(selector),
            } if !selector.trim().is_empty() => Ok(BrowserLocator::css(selector)),
            Self::Structured { .. } => {
                Err("target must provide exactly one of ref or selector".to_string())
            }
        }
    }

    fn key(&self) -> String {
        match self {
            Self::Text(value) => value.clone(),
            Self::Structured {
                reference: Some(reference),
                ..
            } => format!("ref={reference}"),
            Self::Structured {
                selector: Some(selector),
                ..
            } => selector.clone(),
            Self::Structured { .. } => "invalid-target".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ProbeViewport {
    pub width: u32,
    #[serde(default = "default_viewport_height")]
    pub height: u32,
}

fn default_viewport_height() -> u32 {
    DEFAULT_VIEWPORT_HEIGHT
}

#[derive(Clone, Debug, Deserialize)]
pub struct UiProbeArgs {
    pub targets: Vec<TargetInput>,
    pub viewports: Vec<ProbeViewport>,
    #[serde(default = "default_themes")]
    pub themes: Vec<String>,
    #[serde(default = "default_states")]
    pub states: Vec<String>,
    #[serde(default)]
    pub properties: Vec<String>,
}

fn default_themes() -> Vec<String> {
    vec!["light".to_string(), "dark".to_string()]
}

fn default_states() -> Vec<String> {
    vec!["default".to_string()]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeCell {
    pub target: usize,
    pub viewport: usize,
    pub theme: usize,
    pub state: usize,
}

pub fn expand_probe_matrix(args: &UiProbeArgs) -> Result<Vec<ProbeCell>, String> {
    if args.targets.is_empty() || args.targets.len() > MAX_TARGETS {
        return Err(format!("targets must contain 1..={MAX_TARGETS} entries"));
    }
    if args.viewports.is_empty() {
        return Err("viewports must not be empty".to_string());
    }
    if args.themes.is_empty() || args.states.is_empty() {
        return Err("themes and states must not be empty".to_string());
    }
    let total = args
        .targets
        .len()
        .checked_mul(args.viewports.len())
        .and_then(|value| value.checked_mul(args.themes.len()))
        .and_then(|value| value.checked_mul(args.states.len()))
        .ok_or_else(|| "probe matrix is too large".to_string())?;
    if total > MAX_MATRIX_CELLS {
        return Err(format!("probe matrix exceeds {MAX_MATRIX_CELLS} cells"));
    }
    let mut cells = Vec::with_capacity(total);
    for target in 0..args.targets.len() {
        for viewport in 0..args.viewports.len() {
            for theme in 0..args.themes.len() {
                for state in 0..args.states.len() {
                    cells.push(ProbeCell {
                        target,
                        viewport,
                        theme,
                        state,
                    });
                }
            }
        }
    }
    Ok(cells)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl From<&SnapshotBox> for Rect {
    fn from(value: &SnapshotBox) -> Self {
        Self {
            x: value.x as f64,
            y: value.y as f64,
            width: value.width as f64,
            height: value.height as f64,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageArtifact {
    pub kind: String,
    pub mime: String,
    pub data: String,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MarkRecord {
    pub mark_id: usize,
    #[serde(rename = "ref")]
    pub reference: String,
    pub selector: String,
    pub role: String,
    pub name: Option<String>,
    pub rect: Rect,
}

pub fn map_snapshot_marks(nodes: &[SnapshotNode]) -> Vec<MarkRecord> {
    nodes
        .iter()
        .filter_map(|node| Some((node, node.reference.as_ref()?, node.geometry.as_ref()?)))
        .enumerate()
        .map(|(index, (node, reference, geometry))| MarkRecord {
            mark_id: index + 1,
            reference: reference.clone(),
            selector: format!("ref={reference}"),
            role: node.role.clone(),
            name: node.name.clone(),
            rect: geometry.into(),
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

fn parse_color_component(value: &str) -> Result<f64, String> {
    value
        .trim()
        .parse::<f64>()
        .map(|value| value.clamp(0.0, 255.0))
        .map_err(|_| format!("invalid color component `{value}`"))
}

pub fn parse_css_color(value: &str) -> Result<Rgb, String> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let expanded = match hex.len() {
            3 => hex.chars().flat_map(|ch| [ch, ch]).collect::<String>(),
            6 => hex.to_string(),
            _ => return Err(format!("unsupported hex color `{value}`")),
        };
        return Ok(Rgb {
            r: u8::from_str_radix(&expanded[0..2], 16).map_err(|_| "invalid red".to_string())?
                as f64,
            g: u8::from_str_radix(&expanded[2..4], 16).map_err(|_| "invalid green".to_string())?
                as f64,
            b: u8::from_str_radix(&expanded[4..6], 16).map_err(|_| "invalid blue".to_string())?
                as f64,
        });
    }
    let open = value
        .find('(')
        .ok_or_else(|| format!("unsupported color `{value}`"))?;
    let close = value
        .rfind(')')
        .ok_or_else(|| format!("unsupported color `{value}`"))?;
    if !matches!(&value[..open], "rgb" | "rgba") {
        return Err(format!("unsupported color `{value}`"));
    }
    let parts = value[open + 1..close]
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    if parts.len() < 3 {
        return Err(format!("unsupported color `{value}`"));
    }
    Ok(Rgb {
        r: parse_color_component(parts[0])?,
        g: parse_color_component(parts[1])?,
        b: parse_color_component(parts[2])?,
    })
}

fn relative_luminance(color: Rgb) -> f64 {
    fn channel(value: f64) -> f64 {
        let value = value / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
}

pub fn contrast_ratio(foreground: Rgb, background: Rgb) -> f64 {
    let first = relative_luminance(foreground);
    let second = relative_luminance(background);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn text_threshold(font_size: f64, font_weight: &str) -> f64 {
    let weight = font_weight.parse::<u16>().unwrap_or_else(|_| {
        if font_weight.eq_ignore_ascii_case("bold") {
            700
        } else {
            400
        }
    });
    if (font_size >= 18.66 && weight >= 700) || font_size >= 24.0 {
        3.0
    } else {
        4.5
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub fn padded_crop_rect(
    image_width: u32,
    image_height: u32,
    rect: CropRect,
    padding: u32,
) -> Result<CropRect, String> {
    if rect.width == 0 || rect.height == 0 {
        return Err("crop width and height must be positive".to_string());
    }
    if rect.x >= image_width || rect.y >= image_height {
        return Err("crop origin is outside the image".to_string());
    }
    let x = rect.x.saturating_sub(padding);
    let y = rect.y.saturating_sub(padding);
    let right = rect
        .x
        .saturating_add(rect.width)
        .saturating_add(padding)
        .min(image_width);
    let bottom = rect
        .y
        .saturating_add(rect.height)
        .saturating_add(padding)
        .min(image_height);
    if right <= x || bottom <= y {
        return Err("crop does not intersect the image".to_string());
    }
    Ok(CropRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

#[derive(Clone, Debug, Deserialize)]
pub struct ImageRegionArgs {
    pub image_path: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub padding: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiffMask {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VisualDiffArgs {
    pub baseline: String,
    #[serde(default = "default_diff_threshold")]
    pub threshold: f64,
    #[serde(default)]
    pub masks: Vec<DiffMask>,
    #[serde(default)]
    pub update_baseline: bool,
}

fn default_diff_threshold() -> f64 {
    DEFAULT_DIFF_THRESHOLD
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ChangedRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub changed_pixels: u64,
}

#[derive(Clone, Debug)]
pub struct DiffResult {
    pub changed_pixels: u64,
    pub total_pixels: u64,
    pub changed_percent: f64,
    pub regions: Vec<ChangedRegion>,
    pub image: DynamicImage,
}

fn pixel_masked(x: u32, y: u32, masks: &[DiffMask]) -> bool {
    masks.iter().any(|mask| {
        x >= mask.x
            && y >= mask.y
            && x < mask.x.saturating_add(mask.width)
            && y < mask.y.saturating_add(mask.height)
    })
}

fn pixel_changed(first: Rgba<u8>, second: Rgba<u8>, threshold: f64) -> bool {
    first
        .0
        .iter()
        .zip(second.0.iter())
        .map(|(left, right)| (f64::from(*left) - f64::from(*right)).abs() / 255.0)
        .fold(0.0, f64::max)
        > threshold
}

pub fn compare_images(
    baseline: &DynamicImage,
    current: &DynamicImage,
    threshold: f64,
    masks: &[DiffMask],
) -> Result<DiffResult, String> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err("threshold must be between 0 and 1".to_string());
    }
    if baseline.dimensions() != current.dimensions() {
        return Err(format!(
            "image dimensions differ: baseline={}x{}, current={}x{}",
            baseline.width(),
            baseline.height(),
            current.width(),
            current.height()
        ));
    }
    let baseline = baseline.to_rgba8();
    let current = current.to_rgba8();
    let mut changed = vec![false; (baseline.width() * baseline.height()) as usize];
    let mut diff = RgbaImage::new(baseline.width(), baseline.height());
    let mut changed_pixels = 0_u64;
    let mut total_pixels = 0_u64;
    for y in 0..baseline.height() {
        for x in 0..baseline.width() {
            if pixel_masked(x, y, masks) {
                diff.put_pixel(x, y, Rgba([0, 0, 0, 0]));
                continue;
            }
            total_pixels += 1;
            let current_pixel = *current.get_pixel(x, y);
            if pixel_changed(*baseline.get_pixel(x, y), current_pixel, threshold) {
                changed[(y * baseline.width() + x) as usize] = true;
                changed_pixels += 1;
                diff.put_pixel(x, y, Rgba([255, 0, 255, 255]));
            } else {
                diff.put_pixel(
                    x,
                    y,
                    Rgba([
                        current_pixel[0] / 3,
                        current_pixel[1] / 3,
                        current_pixel[2] / 3,
                        180,
                    ]),
                );
            }
        }
    }
    let regions = changed_regions(&changed, baseline.width(), baseline.height());
    Ok(DiffResult {
        changed_pixels,
        total_pixels,
        changed_percent: if total_pixels == 0 {
            0.0
        } else {
            changed_pixels as f64 * 100.0 / total_pixels as f64
        },
        regions,
        image: DynamicImage::ImageRgba8(diff),
    })
}

fn changed_regions(changed: &[bool], width: u32, height: u32) -> Vec<ChangedRegion> {
    let mut visited = vec![false; changed.len()];
    let mut regions = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            if !changed[index] || visited[index] {
                continue;
            }
            let mut queue = VecDeque::from([(x, y)]);
            visited[index] = true;
            let (mut min_x, mut max_x, mut min_y, mut max_y) = (x, x, y, y);
            let mut pixels = 0_u64;
            while let Some((cx, cy)) = queue.pop_front() {
                pixels += 1;
                min_x = min_x.min(cx);
                max_x = max_x.max(cx);
                min_y = min_y.min(cy);
                max_y = max_y.max(cy);
                for (nx, ny) in [
                    (cx.wrapping_sub(1), cy),
                    (cx.saturating_add(1), cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy.saturating_add(1)),
                ] {
                    if nx >= width || ny >= height {
                        continue;
                    }
                    let next = (ny * width + nx) as usize;
                    if changed[next] && !visited[next] {
                        visited[next] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }
            regions.push(ChangedRegion {
                x: min_x,
                y: min_y,
                width: max_x - min_x + 1,
                height: max_y - min_y + 1,
                changed_pixels: pixels,
            });
        }
    }
    regions.sort_by(|left, right| right.changed_pixels.cmp(&left.changed_pixels));
    regions
}

#[derive(Clone, Debug, Deserialize)]
pub struct ContrastAuditArgs {
    #[serde(default)]
    pub token_files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawContrastSample {
    kind: String,
    selector: String,
    text: String,
    foreground: String,
    background: String,
    font_size: f64,
    font_weight: String,
}

#[derive(Clone, Debug, Serialize)]
struct ContrastFinding {
    selector: String,
    text: String,
    foreground: String,
    background: String,
    ratio: f64,
    threshold: f64,
    aa: bool,
    aaa: bool,
    severity: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct RawColorFinding {
    color: String,
    selector: String,
    severity: &'static str,
}

fn tool_message(tool_call_id: &str, text: String) -> Result<(bool, Vec<ContextEnum>), String> {
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        })],
    ))
}

async fn tool_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
) -> (
    crate::app_state::AppState,
    String,
    String,
    Option<crate::worktrees::scope::ExecutionScope>,
) {
    let ccx = ccx.lock().await;
    (
        ccx.app.clone(),
        ccx.chat_id.clone(),
        ccx.current_model.clone(),
        ccx.execution_scope.clone(),
    )
}

async fn attached_runtime(
    app: crate::app_state::AppState,
    chat_id: &str,
) -> Result<Arc<AMutex<BrowserRuntime>>, String> {
    find_runtime_by_chat_id(app, chat_id)
        .await
        .map(|(_, runtime)| runtime)
        .ok_or_else(|| format!("No browser runtime is attached to chat `{chat_id}`"))
}

async fn image_policy_for_model(
    gcx: Arc<crate::global_context::GlobalContext>,
    model_id: &str,
) -> ImagePolicy {
    let Ok(caps) = crate::global_context::try_load_caps_quickly_if_not_present(gcx, 0).await else {
        return ImagePolicy::default();
    };
    crate::caps::resolve_chat_model(caps, model_id)
        .map(|model| ImagePolicy::for_model(&model.base))
        .unwrap_or_default()
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: &HashMap<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(args.clone().into_iter().collect()))
        .map_err(|error| format!("invalid arguments: {error}"))
}

fn tool_desc(
    config_path: &str,
    name: &str,
    display_name: &str,
    description: &str,
    input_schema: Value,
    destructive: bool,
) -> ToolDesc {
    ToolDesc {
        name: name.to_string(),
        display_name: display_name.to_string(),
        source: ToolSource {
            source_type: ToolSourceType::Builtin,
            config_path: config_path.to_string(),
        },
        experimental: false,
        allow_parallel: false,
        description: description.to_string(),
        input_schema,
        output_schema: None,
        annotations: Some(json!({
            "readOnlyHint": !destructive,
            "destructiveHint": destructive,
            "idempotentHint": !destructive,
            "openWorldHint": true
        })),
    }
}

fn locator_value(locator: &BrowserLocator) -> Result<Value, String> {
    serde_json::to_value(locator).map_err(|error| format!("failed to serialize locator: {error}"))
}

pub fn map_design_runtime_error(error: &str) -> String {
    if INSTRUMENTATION_ERROR_MARKERS
        .iter()
        .any(|marker| error.contains(marker))
    {
        return PAGE_NOT_INSTRUMENTED_ERROR.to_string();
    }
    error.to_string()
}

fn resolve_target_handle(
    tab: &Tab,
    world: &WorldManager,
    target: &TargetInput,
) -> Result<refact_browser::ElementHandle, String> {
    let locator = target.locator()?;
    let handles = world
        .call_injected_handles(tab, "resolveLocator", json!([locator_value(&locator)?]))
        .map_err(|error| {
            map_design_runtime_error(&format!("failed to resolve {}: {error}", target.key()))
        })?;
    match handles.as_slice() {
        [handle] => Ok(handle.clone()),
        [] => Err(format!("target `{}` matched no elements", target.key())),
        handles => Err(format!(
            "target `{}` matched {} elements; use a unique selector or ref",
            target.key(),
            handles.len()
        )),
    }
}

fn apply_probe_state(
    tab: &Tab,
    world: &WorldManager,
    handle: &refact_browser::ElementHandle,
    state: &str,
) -> Result<(), String> {
    match state {
        "default" => Ok(()),
        "hover" => {
            let rect: Rect = serde_json::from_value(
                world
                    .call_function_on(tab, handle, "function(){const r=this.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};}", vec![])
                    .map_err(|error| format!("failed to read hover target: {error}"))?,
            )
            .map_err(|error| format!("failed to parse hover target: {error}"))?;
            tab.call_method(Input::DispatchMouseEvent {
                Type: Input::DispatchMouseEventTypeOption::MouseMoved,
                x: rect.x + rect.width / 2.0,
                y: rect.y + rect.height / 2.0,
                modifiers: Some(0),
                timestamp: None,
                button: None,
                buttons: Some(0),
                click_count: None,
                force: None,
                tangential_pressure: None,
                tilt_x: None,
                tilt_y: None,
                twist: None,
                delta_x: None,
                delta_y: None,
                pointer_Type: None,
            })
            .map(|_| ())
            .map_err(|error| format!("failed to hover target: {error}"))
        }
        "focus" => world
            .call_function_on(tab, handle, "function(){this.focus();return true;}", vec![])
            .map(|_| ())
            .map_err(|error| format!("failed to focus target: {error}")),
        "active" => {
            let rect: Rect = serde_json::from_value(
                world
                    .call_function_on(tab, handle, "function(){const r=this.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};}", vec![])
                    .map_err(|error| format!("failed to read active target: {error}"))?,
            )
            .map_err(|error| format!("failed to parse active target: {error}"))?;
            let x = rect.x + rect.width / 2.0;
            let y = rect.y + rect.height / 2.0;
            for event_type in [
                Input::DispatchMouseEventTypeOption::MouseMoved,
                Input::DispatchMouseEventTypeOption::MousePressed,
            ] {
                tab.call_method(Input::DispatchMouseEvent {
                    Type: event_type,
                    x,
                    y,
                    modifiers: Some(0),
                    timestamp: None,
                    button: Some(Input::MouseButton::Left),
                    buttons: Some(1),
                    click_count: Some(1),
                    force: Some(0.5),
                    tangential_pressure: None,
                    tilt_x: None,
                    tilt_y: None,
                    twist: None,
                    delta_x: None,
                    delta_y: None,
                    pointer_Type: None,
                })
                .map_err(|error| format!("failed to activate target: {error}"))?;
            }
            Ok(())
        }
        other => Err(format!(
            "unsupported state `{other}`; expected default, hover, focus, or active"
        )),
    }
}

fn cleanup_probe_state(tab: &Tab, world: &WorldManager) {
    let _ = world.eval_in_utility(tab, "document.activeElement?.blur?.();true");
    let _ = tab.call_method(Input::DispatchMouseEvent {
        Type: Input::DispatchMouseEventTypeOption::MouseReleased,
        x: -10.0,
        y: -10.0,
        modifiers: Some(0),
        timestamp: None,
        button: Some(Input::MouseButton::Left),
        buttons: Some(0),
        click_count: Some(1),
        force: None,
        tangential_pressure: None,
        tilt_x: None,
        tilt_y: None,
        twist: None,
        delta_x: None,
        delta_y: None,
        pointer_Type: None,
    });
    let _ = tab.call_method(Input::DispatchMouseEvent {
        Type: Input::DispatchMouseEventTypeOption::MouseMoved,
        x: -10.0,
        y: -10.0,
        modifiers: Some(0),
        timestamp: None,
        button: None,
        buttons: Some(0),
        click_count: None,
        force: None,
        tangential_pressure: None,
        tilt_x: None,
        tilt_y: None,
        twist: None,
        delta_x: None,
        delta_y: None,
        pointer_Type: None,
    });
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|error| format!("failed to encode PNG: {error}"))?;
    Ok(bytes)
}

fn artifact_from_bytes(
    bytes: Vec<u8>,
    mime: &str,
    policy: &ImagePolicy,
) -> Result<ImageArtifact, String> {
    let (processed, mime) = resize_to_policy(&bytes, mime, policy)?;
    let decoded = image::load_from_memory(&processed)
        .map_err(|error| format!("failed to decode processed image: {error}"))?;
    Ok(ImageArtifact {
        kind: "image".to_string(),
        mime,
        data: base64::prelude::BASE64_STANDARD.encode(&processed),
        width: decoded.width(),
        height: decoded.height(),
        bytes: processed.len(),
    })
}

fn screenshot_artifact_from_step(data: &Value) -> Result<ImageArtifact, String> {
    let artifact = data.get("artifact").unwrap_or(data);
    serde_json::from_value(artifact.clone())
        .map_err(|error| format!("failed to parse screenshot artifact: {error}"))
}

fn capture_runtime_screenshot(
    runtime: &mut BrowserRuntime,
    policy: &ImagePolicy,
    mask: Vec<BrowserLocator>,
) -> Result<ImageArtifact, String> {
    let report = browser_controller::execute_steps_with_runtime(
        runtime,
        &[BrowserStep::Screenshot {
            options: BrowserScreenshotOptions {
                image_type: Some(BrowserScreenshotType::Png),
                animations: Some(BrowserScreenshotAnimations::Disabled),
                caret: Some(BrowserScreenshotCaret::Hide),
                mask,
                ..Default::default()
            },
        }],
        policy,
    );
    if !report.ok {
        return Err(report
            .steps
            .last()
            .and_then(|step| step.error.clone())
            .unwrap_or_else(|| "screenshot capture failed".to_string()));
    }
    screenshot_artifact_from_step(
        report
            .steps
            .first()
            .and_then(|step| step.data.as_ref())
            .ok_or_else(|| "screenshot result had no artifact".to_string())?,
    )
}

fn default_page_text_masks() -> Vec<BrowserLocator> {
    vec![BrowserLocator::css(
        "input[type=password], textarea[data-private], [data-refact-mask], [data-private]",
    )]
}

fn install_mark_overlay(
    tab: &Tab,
    world: &WorldManager,
    marks: &[MarkRecord],
) -> Result<(), String> {
    let marks = serde_json::to_string(marks).map_err(|error| error.to_string())?;
    let script = format!(
        r#"(() => {{
  window.__refactHideMarks?.();
  const root = document.createElement('div');
  root.dataset.refactMarks = 'true';
  const shadow = root.attachShadow({{mode:'closed'}});
  for (const mark of {marks}) {{
    const badge = document.createElement('div');
    badge.textContent = String(mark.mark_id);
    Object.assign(badge.style, {{position:'fixed',left:mark.rect.x+'px',top:mark.rect.y+'px',minWidth:'18px',height:'18px',padding:'0 3px',font:'bold 12px/18px sans-serif',textAlign:'center',color:'white',background:'#E7150D',border:'1px solid white',borderRadius:'9px',boxSizing:'border-box',pointerEvents:'none',zIndex:'2147483647'}});
    shadow.appendChild(badge);
  }}
  document.documentElement.appendChild(root);
  window.__refactHideMarks = () => {{ root.remove(); delete window.__refactHideMarks; }};
  return true;
}})()"#
    );
    world
        .eval_in_utility(tab, &script)
        .map(|_| ())
        .map_err(|error| format!("failed to install mark overlay: {error}"))
}

fn find_raw_colors(tab: &Tab, token_colors: &[String]) -> Result<Vec<RawColorFinding>, String> {
    let token_colors = serde_json::to_string(token_colors).map_err(|error| error.to_string())?;
    let expression = format!(
        r#"(() => {{
  const known = new Set({token_colors}.map(value => value.toLowerCase()));
  const findings = [];
  const hex = /#[0-9a-fA-F]{{3,8}}\b/g;
  for (const sheet of Array.from(document.styleSheets)) {{
    let rules;
    try {{ rules = Array.from(sheet.cssRules || []); }} catch {{ continue; }}
    for (const rule of rules) {{
      const text = rule.cssText || '';
      for (const color of text.match(hex) || []) {{
        if (!known.has(color.toLowerCase())) findings.push({{color,selector:rule.selectorText || '[stylesheet]'}});
      }}
    }}
  }}
  return findings;
}})()"#
    );
    let value = tab
        .evaluate(&expression, true)
        .map_err(|error| format!("failed to inspect stylesheet colors: {error}"))?
        .value
        .unwrap_or(Value::Array(Vec::new()));
    let values: Vec<Value> = serde_json::from_value(value)
        .map_err(|error| format!("failed to parse stylesheet colors: {error}"))?;
    Ok(values
        .into_iter()
        .filter_map(|value| {
            Some(RawColorFinding {
                color: value.get("color")?.as_str()?.to_string(),
                selector: value.get("selector")?.as_str()?.to_string(),
                severity: "Low",
            })
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenColorScan {
    pub colors: Vec<String>,
    pub resolved_files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContrastAuditVerdict {
    pub summary: String,
    pub warning: Option<String>,
}

pub fn contrast_audit_verdict(
    elements_scanned: usize,
    failed: usize,
    aaa_warnings: usize,
    raw_colors: usize,
    resolved_token_files: usize,
) -> ContrastAuditVerdict {
    let mut warnings = Vec::new();
    if elements_scanned == 0 {
        warnings.push(ZERO_SCAN_WARNING);
    }
    if resolved_token_files == 0 {
        warnings.push(NO_TOKEN_FILES_WARNING);
    }
    let measured = format!(
        "Contrast audit scanned {elements_scanned} elements: {failed} AA failures, {aaa_warnings} AAA warnings, and {raw_colors} non-token colors"
    );
    if warnings.is_empty() {
        return ContrastAuditVerdict {
            summary: measured,
            warning: None,
        };
    }
    let warning = warnings.join(" | ");
    ContrastAuditVerdict {
        summary: format!("{warning}. {measured}"),
        warning: Some(warning),
    }
}

fn token_colors_from_files(root: &Path, token_files: &[String]) -> TokenColorScan {
    let mut colors = Vec::new();
    let mut resolved_files = Vec::new();
    for relative in token_files {
        let path = root.join(relative);
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        resolved_files.push(relative.clone());
        let bytes = content.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'#' {
                let start = index;
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_hexdigit() {
                    index += 1;
                }
                let len = index - start - 1;
                if matches!(len, 3 | 4 | 6 | 8) {
                    colors.push(content[start..index].to_ascii_lowercase());
                }
            } else {
                index += 1;
            }
        }
    }
    colors.sort();
    colors.dedup();
    TokenColorScan {
        colors,
        resolved_files,
    }
}

fn project_root(
    gcx: &crate::global_context::GlobalContext,
    execution_scope: Option<&crate::worktrees::scope::ExecutionScope>,
) -> Result<PathBuf, String> {
    if let Some(scope) = execution_scope {
        scope.ensure_active_root()?;
        return Ok(scope.effective_root().to_path_buf());
    }
    gcx.documents_state
        .workspace_folders
        .lock()
        .map_err(|error| error.to_string())?
        .first()
        .cloned()
        .ok_or_else(|| "No workspace root is available".to_string())
}

fn baseline_path(root: &Path, requested: &str) -> Result<PathBuf, String> {
    if requested.trim().is_empty() {
        return Err("baseline must not be empty".to_string());
    }
    let relative = Path::new(requested);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("baseline must be a repository-relative path".to_string());
    }
    let relative = if relative.starts_with(".refact") {
        relative.to_path_buf()
    } else {
        PathBuf::from(".refact")
            .join("visual_baselines")
            .join(relative)
    };
    let path = root.join(relative);
    if !path.starts_with(root.join(".refact")) {
        return Err("visual baselines must live under .refact".to_string());
    }
    Ok(path)
}

fn image_region_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "image_path":{"type":"string","description":"Path to an already-captured image"},
            "x":{"type":"integer","minimum":0},
            "y":{"type":"integer","minimum":0},
            "width":{"type":"integer","minimum":1},
            "height":{"type":"integer","minimum":1},
            "padding":{"type":"integer","minimum":0,"default":0}
        },
        "required":["image_path","x","y","width","height"]
    })
}

pub struct ToolUiProbe {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolUiProbe {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let args: UiProbeArgs = parse_args(args)?;
        let cells = expand_probe_matrix(&args)?;
        for viewport in &args.viewports {
            if viewport.width == 0 || viewport.height == 0 {
                return Err("viewport dimensions must be positive".to_string());
            }
        }
        for theme in &args.themes {
            if !matches!(theme.as_str(), "light" | "dark") {
                return Err(format!(
                    "unsupported theme `{theme}`; expected light or dark"
                ));
            }
        }
        let (app, chat_id, _, _) = tool_context(&ccx).await;
        let runtime = attached_runtime(app, &chat_id).await?;
        let mut runtime = runtime.lock().await;
        let tab = runtime
            .get_active_tab()
            .ok_or_else(|| "The attached browser has no active tab".to_string())?;
        let original_viewport = runtime.context_state.viewport.clone();
        let original_media = runtime.context_state.media.clone();
        let properties = if args.properties.is_empty() {
            DEFAULT_STYLE_PROPERTIES
                .iter()
                .map(|value| value.to_string())
                .collect()
        } else {
            args.properties.clone()
        };
        let mut table = Vec::with_capacity(cells.len());
        for cell in cells {
            let viewport = &args.viewports[cell.viewport];
            let theme = &args.themes[cell.theme];
            let state = &args.states[cell.state];
            let viewport_state = ViewportState {
                width: viewport.width,
                height: viewport.height,
                device_scale_factor: 1.0,
                is_mobile: false,
                has_touch: false,
            };
            refact_browser::context_state::apply_viewport(&tab, &viewport_state)?;
            refact_browser::context_state::apply_media(
                &tab,
                &MediaState {
                    color_scheme: Some(theme.clone()),
                    ..Default::default()
                },
            )?;
            cleanup_probe_state(&tab, &runtime.world_manager);
            let target = &args.targets[cell.target];
            let handle = resolve_target_handle(&tab, &runtime.world_manager, target)?;
            apply_probe_state(&tab, &runtime.world_manager, &handle, state)?;
            let data = runtime
                .world_manager
                .call_function_on(
                    &tab,
                    &handle,
                    PROBE_FUNCTION,
                    vec![serde_json::to_value(&properties).map_err(|error| error.to_string())?],
                )
                .map_err(|error| format!("failed to probe {}: {error}", target.key()))?;
            table.push(json!({
                "target": target.key(),
                "viewport": {"width":viewport.width,"height":viewport.height},
                "theme": theme,
                "state": state,
                "box": data.get("box").cloned().unwrap_or(Value::Null),
                "styles": data.get("styles").cloned().unwrap_or(Value::Null),
                "overflow": data.get("overflow").cloned().unwrap_or(Value::Null)
            }));
            let _ = runtime.world_manager.release_handle(&tab, &handle);
        }
        cleanup_probe_state(&tab, &runtime.world_manager);
        if let Some(viewport) = original_viewport.as_ref() {
            let _ = refact_browser::context_state::apply_viewport(&tab, viewport);
        } else {
            let _ = tab.call_method(Emulation::ClearDeviceMetricsOverride(None));
        }
        let _ = refact_browser::context_state::apply_media(&tab, &original_media);
        runtime.touch();
        let summary = format!(
            "Probed {} target-state combinations without screenshots",
            table.len()
        );
        tool_message(
            tool_call_id,
            ToolJson::new(
                "ui_probe",
                summary,
                json!({
                    "matrix": table,
                    "target_count":args.targets.len(),
                    "viewport_count":args.viewports.len(),
                    "theme_count":args.themes.len(),
                    "state_count":args.states.len()
                }),
            )
            .to_text(),
        )
    }

    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            &self.config_path,
            "ui_probe",
            "UI Probe",
            "Measure live DOM targets across a viewport × theme × state matrix. Returns computed styles, rectangles, and overflow flags without screenshots. Requires a page instrumented by the local dev-server flow; third-party pages return a page-not-instrumented error.",
            json!({
                "type":"object",
                "properties":{
                    "targets":{"type":"array","minItems":1,"maxItems":MAX_TARGETS,"items":{"oneOf":[{"type":"string"},{"type":"object","properties":{"ref":{"type":"string"},"selector":{"type":"string"}}}]}},
                    "viewports":{"type":"array","minItems":1,"items":{"type":"object","properties":{"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1,"default":DEFAULT_VIEWPORT_HEIGHT}},"required":["width"]}},
                    "themes":{"type":"array","items":{"type":"string","enum":["light","dark"]},"default":["light","dark"]},
                    "states":{"type":"array","items":{"type":"string","enum":["default","hover","focus","active"]},"default":["default"]},
                    "properties":{"type":"array","items":{"type":"string"}}
                },
                "required":["targets","viewports"]
            }),
            false,
        )
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

pub struct ToolMarkElements {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolMarkElements {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let selector = args.get("selector").and_then(Value::as_str);
        let (app, chat_id, model_id, _) = tool_context(&ccx).await;
        let policy = image_policy_for_model(app.gcx.clone(), &model_id).await;
        let runtime = attached_runtime(app, &chat_id).await?;
        let mut runtime = runtime.lock().await;
        let tab = runtime
            .get_active_tab()
            .ok_or_else(|| "The attached browser has no active tab".to_string())?;
        let root_handle = selector
            .map(|selector| {
                runtime.world_manager.resolve_expression_handle(
                    &tab,
                    &format!(
                        "document.querySelector({})",
                        serde_json::to_string(selector).unwrap()
                    ),
                )
            })
            .transpose()
            .map_err(|error| format!("failed to resolve mark root: {error}"))?;
        let snapshot = runtime
            .world_manager
            .aria_snapshot(
                &tab,
                root_handle.clone(),
                SnapshotOptions {
                    refs: true,
                    boxes: true,
                    ..Default::default()
                },
            )
            .map_err(|error| format!("failed to snapshot marks: {error}"))?;
        if let Some(handle) = root_handle {
            let _ = runtime.world_manager.release_handle(&tab, &handle);
        }
        let mut marks = map_snapshot_marks(&snapshot.nodes);
        for mark in &mut marks {
            let reference: refact_browser::Ref = mark
                .reference
                .parse()
                .map_err(|error| format!("failed to parse mark ref: {error}"))?;
            let handle = runtime
                .world_manager
                .resolve_ref(&tab, &reference)
                .map_err(|error| format!("failed to resolve mark ref: {error}"))?;
            mark.selector = runtime
                .world_manager
                .generate_locator(&tab, &handle, Default::default())
                .unwrap_or_else(|_| format!("ref={}", mark.reference));
            let _ = runtime.world_manager.release_handle(&tab, &handle);
        }
        install_mark_overlay(&tab, &runtime.world_manager, &marks)?;
        let artifact =
            capture_runtime_screenshot(&mut runtime, &policy, default_page_text_masks())?;
        let _ = runtime
            .world_manager
            .eval_in_utility(&tab, "window.__refactHideMarks?.(); true");
        runtime.touch();
        let summary = format!("Marked {} DOM-exact ARIA elements", marks.len());
        tool_message(
            tool_call_id,
            ToolJson::new(
                "mark_elements",
                summary,
                json!({"marks":marks,"artifact":artifact}),
            )
            .to_text(),
        )
    }

    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            &self.config_path,
            "mark_elements",
            "Mark Elements",
            "Create exact numbered Set-of-Mark badges from the live ARIA snapshot and DOM rectangles, returning mark-to-ref mappings and a masked screenshot.",
            json!({"type":"object","properties":{"selector":{"type":"string","description":"Optional CSS selector limiting the marked subtree"}}}),
            false,
        )
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

pub struct ToolContrastAudit {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolContrastAudit {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let args: ContrastAuditArgs = parse_args(args)?;
        let (app, chat_id, _, execution_scope) = tool_context(&ccx).await;
        let root = project_root(&app.gcx, execution_scope.as_ref())?;
        let token_files = if args.token_files.is_empty() {
            DEFAULT_DESIGN_TOKEN_STYLES
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        } else {
            args.token_files
        };
        let token_scan = token_colors_from_files(&root, &token_files);
        let runtime = attached_runtime(app, &chat_id).await?;
        let mut runtime = runtime.lock().await;
        let tab = runtime
            .get_active_tab()
            .ok_or_else(|| "The attached browser has no active tab".to_string())?;
        let raw = tab
            .evaluate(CONTRAST_AUDIT_EXPRESSION, true)
            .map_err(|error| format!("failed to audit contrast: {error}"))?
            .value
            .unwrap_or(Value::Array(Vec::new()));
        let samples: Vec<RawContrastSample> = serde_json::from_value(raw)
            .map_err(|error| format!("failed to parse contrast samples: {error}"))?;
        let elements_scanned = samples.len();
        let mut findings = Vec::new();
        for sample in samples {
            let ratio = contrast_ratio(
                parse_css_color(&sample.foreground)?,
                parse_css_color(&sample.background)?,
            );
            let threshold = if sample.kind == "non_text" {
                3.0
            } else {
                text_threshold(sample.font_size, &sample.font_weight)
            };
            if ratio < 7.0 {
                findings.push(ContrastFinding {
                    selector: sample.selector,
                    text: sample.text,
                    foreground: sample.foreground,
                    background: sample.background,
                    ratio: (ratio * 100.0).round() / 100.0,
                    threshold,
                    aa: ratio >= threshold,
                    aaa: ratio >= 7.0,
                    severity: if ratio < threshold { "High" } else { "Medium" },
                });
            }
            if findings.len() >= MAX_AUDIT_FINDINGS {
                break;
            }
        }
        let raw_colors = find_raw_colors(&tab, &token_scan.colors)?;
        runtime.touch();
        let failed = findings.iter().filter(|finding| !finding.aa).count();
        let verdict = contrast_audit_verdict(
            elements_scanned,
            failed,
            findings.len().saturating_sub(failed),
            raw_colors.len(),
            token_scan.resolved_files.len(),
        );
        tool_message(
            tool_call_id,
            ToolJson::new(
                "contrast_audit",
                verdict.summary,
                json!({
                    "findings":findings,
                    "raw_colors":raw_colors,
                    "thresholds":{"aaa":7.0,"aa":4.5,"large_text":3.0,"non_text":3.0},
                    "token_files":token_files,
                    "token_files_resolved":token_scan.resolved_files,
                    "token_color_count":token_scan.colors.len(),
                    "elements_scanned":elements_scanned,
                    "warning":verdict.warning
                }),
            )
            .to_text(),
        )
    }

    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            &self.config_path,
            "contrast_audit",
            "Contrast Audit",
            "Audit live DOM text contrast against WCAG AAA 7.0, AA 4.5, large-text 3.0, and non-text 3.0 thresholds; also report raw stylesheet colors absent from discovered token files. Fails closed: reports elements_scanned and, when zero elements were measured or no token file resolved, leads the summary with a warning instead of a pass.",
            json!({"type":"object","properties":{"token_files":{"type":"array","items":{"type":"string"},"description":"Repository-relative design-token CSS files"}}}),
            false,
        )
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

pub struct ToolImageRegion {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolImageRegion {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let args: ImageRegionArgs = parse_args(args)?;
        let (app, _, model_id, execution_scope) = tool_context(&ccx).await;
        let policy = image_policy_for_model(app.gcx.clone(), &model_id).await;
        let path = if let Some(scope) = execution_scope.as_ref() {
            scope
                .resolve_existing_path(Path::new(&args.image_path))?
                .path
        } else {
            PathBuf::from(&args.image_path)
        };
        crate::files_in_workspace::check_file_privacy_for_send(app.gcx.clone(), &path).await?;
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| format!("failed to read image `{}`: {error}", path.display()))?;
        let image = image::load_from_memory(&bytes)
            .map_err(|error| format!("failed to decode image `{}`: {error}", path.display()))?;
        let rect = padded_crop_rect(
            image.width(),
            image.height(),
            CropRect {
                x: args.x,
                y: args.y,
                width: args.width,
                height: args.height,
            },
            args.padding,
        )?;
        let cropped = image.crop_imm(rect.x, rect.y, rect.width, rect.height);
        let artifact = artifact_from_bytes(encode_png(&cropped)?, "image/png", &policy)?;
        let summary = format!(
            "Cropped {}x{} region from `{}`",
            rect.width,
            rect.height,
            path.display()
        );
        tool_message(
            tool_call_id,
            ToolJson::new(
                "image_region",
                summary,
                json!({"source":path,"region":rect,"artifact":artifact}),
            )
            .to_text(),
        )
    }

    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            &self.config_path,
            "image_region",
            "Image Region",
            "Crop an already-captured image at native resolution with optional padding. Returns a policy-processed image artifact.",
            image_region_schema(),
            false,
        )
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

pub struct ToolVisualDiff {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolVisualDiff {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let args: VisualDiffArgs = parse_args(args)?;
        let (app, chat_id, model_id, execution_scope) = tool_context(&ccx).await;
        let root = project_root(&app.gcx, execution_scope.as_ref())?;
        let baseline = baseline_path(&root, &args.baseline)?;
        let policy = image_policy_for_model(app.gcx.clone(), &model_id).await;
        let runtime = attached_runtime(app, &chat_id).await?;
        let mut runtime = runtime.lock().await;
        let current = capture_runtime_screenshot(
            &mut runtime,
            &ImagePolicy::browser_capture(),
            default_page_text_masks(),
        )?;
        let current_bytes = base64::prelude::BASE64_STANDARD
            .decode(&current.data)
            .map_err(|error| format!("failed to decode current screenshot: {error}"))?;
        if !baseline.exists() || args.update_baseline {
            if !args.update_baseline {
                return Err(format!(
                    "Baseline `{}` does not exist; pass update_baseline: true to create it",
                    baseline.display()
                ));
            }
            if let Some(parent) = baseline.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| format!("failed to create baseline directory: {error}"))?;
            }
            tokio::fs::write(&baseline, &current_bytes)
                .await
                .map_err(|error| format!("failed to write baseline: {error}"))?;
            runtime.touch();
            return tool_message(
                tool_call_id,
                ToolJson::new(
                    "visual_diff",
                    format!("Visual baseline updated at `{}`", baseline.display()),
                    json!({"baseline":baseline,"baseline_updated":true,"changed_pixels":0,"changed_percent":0.0,"regions":[],"artifact":current}),
                )
                .to_text(),
            );
        }
        let baseline_bytes = tokio::fs::read(&baseline)
            .await
            .map_err(|error| format!("failed to read baseline: {error}"))?;
        let baseline_image = image::load_from_memory(&baseline_bytes)
            .map_err(|error| format!("failed to decode baseline: {error}"))?;
        let current_image = image::load_from_memory(&current_bytes)
            .map_err(|error| format!("failed to decode current screenshot: {error}"))?;
        let diff = compare_images(&baseline_image, &current_image, args.threshold, &args.masks)?;
        let artifact = artifact_from_bytes(encode_png(&diff.image)?, "image/png", &policy)?;
        let summary = format!(
            "Visual diff changed {} of {} pixels ({:.3}%)",
            diff.changed_pixels, diff.total_pixels, diff.changed_percent
        );
        runtime.touch();
        tool_message(
            tool_call_id,
            ToolJson::new(
                "visual_diff",
                summary,
                json!({
                    "baseline":baseline,
                    "baseline_updated":false,
                    "threshold":args.threshold,
                    "changed_pixels":diff.changed_pixels,
                    "total_pixels":diff.total_pixels,
                    "changed_percent":diff.changed_percent,
                    "regions":diff.regions,
                    "masks":args.masks,
                    "artifact":artifact
                }),
            )
            .to_text(),
        )
    }

    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            &self.config_path,
            "visual_diff",
            "Visual Diff",
            "Compare the masked live-page screenshot with an explicit baseline stored under .refact/. Never overwrites a baseline unless update_baseline is true; returns pixel statistics, changed regions, and a policy-processed diff artifact.",
            json!({
                "type":"object",
                "properties":{
                    "baseline":{"type":"string","description":"Repository-relative baseline name or .refact path"},
                    "threshold":{"type":"number","minimum":0,"maximum":1,"default":DEFAULT_DIFF_THRESHOLD},
                    "masks":{"type":"array","items":{"type":"object","properties":{"x":{"type":"integer","minimum":0},"y":{"type":"integer","minimum":0},"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1}},"required":["x","y","width","height"]}},
                    "update_baseline":{"type":"boolean","default":false,"description":"Explicitly create or overwrite the baseline"}
                },
                "required":["baseline"]
            }),
            true,
        )
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok(
            if args
                .get("update_baseline")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "update_baseline".to_string()
            } else {
                String::new()
            },
        )
    }

    fn confirm_deny_rules(
        &self,
    ) -> Option<crate::integrations::integr_abstract::IntegrationConfirmation> {
        Some(
            crate::integrations::integr_abstract::IntegrationConfirmation {
                ask_user: vec!["update_baseline".to_string()],
                deny: Vec::new(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_math_matches_wcag_known_pairs_and_edges() {
        assert!(
            (contrast_ratio(
                parse_css_color("#000").unwrap(),
                parse_css_color("rgb(255,255,255)").unwrap()
            ) - 21.0)
                .abs()
                < 0.001
        );
        assert!(
            (contrast_ratio(
                parse_css_color("#777777").unwrap(),
                parse_css_color("#ffffff").unwrap()
            ) - 4.478)
                .abs()
                < 0.01
        );
        assert_eq!(text_threshold(16.0, "400"), 4.5);
        assert_eq!(text_threshold(18.66, "700"), 3.0);
        assert!(parse_css_color("transparent").is_err());
    }

    #[test]
    fn contrast_audit_never_passes_without_measured_elements_or_tokens() {
        let nothing_scanned = contrast_audit_verdict(0, 0, 0, 0, 1);
        assert_eq!(nothing_scanned.warning.as_deref(), Some(ZERO_SCAN_WARNING));
        assert!(nothing_scanned.summary.starts_with(ZERO_SCAN_WARNING));

        let no_tokens = contrast_audit_verdict(12, 0, 0, 0, 0);
        assert_eq!(no_tokens.warning.as_deref(), Some(NO_TOKEN_FILES_WARNING));
        assert!(no_tokens.summary.starts_with(NO_TOKEN_FILES_WARNING));

        let blind = contrast_audit_verdict(0, 0, 0, 0, 0);
        let blind_warning = blind.warning.unwrap();
        assert!(blind_warning.contains(ZERO_SCAN_WARNING));
        assert!(blind_warning.contains(NO_TOKEN_FILES_WARNING));
        assert!(blind.summary.starts_with(ZERO_SCAN_WARNING));

        let clean = contrast_audit_verdict(12, 0, 3, 0, 2);
        assert_eq!(clean.warning, None);
        assert_eq!(
            clean.summary,
            "Contrast audit scanned 12 elements: 0 AA failures, 3 AAA warnings, and 0 non-token colors"
        );
    }

    #[test]
    fn injected_runtime_failures_report_missing_instrumentation() {
        assert_eq!(
            map_design_runtime_error(
                "failed to resolve #root: Browser utility-world evaluation failed: Unknown RefactInjected method"
            ),
            PAGE_NOT_INSTRUMENTED_ERROR
        );
        assert_eq!(
            map_design_runtime_error("failed to resolve #root: RefactInjected is not installed"),
            PAGE_NOT_INSTRUMENTED_ERROR
        );
        assert_eq!(
            map_design_runtime_error("target `#root` matched no elements"),
            "target `#root` matched no elements"
        );
    }

    #[test]
    fn token_scan_reports_only_readable_token_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/styles")).unwrap();
        std::fs::write(
            root.path().join("src/styles/tokens.css"),
            ":root{--accent:#E7150D;--bg:#FFF;}",
        )
        .unwrap();
        let scan = token_colors_from_files(
            root.path(),
            &[
                "src/styles/tokens.css".to_string(),
                "src/styles/missing.css".to_string(),
            ],
        );
        assert_eq!(scan.resolved_files, vec!["src/styles/tokens.css"]);
        assert_eq!(scan.colors, vec!["#e7150d", "#fff"]);

        let empty = token_colors_from_files(root.path(), &["nope.css".to_string()]);
        assert!(empty.resolved_files.is_empty());
        assert!(empty.colors.is_empty());
    }

    #[test]
    fn region_crop_clamps_padding_to_native_bounds() {
        assert_eq!(
            padded_crop_rect(
                100,
                80,
                CropRect {
                    x: 5,
                    y: 6,
                    width: 30,
                    height: 20
                },
                10
            )
            .unwrap(),
            CropRect {
                x: 0,
                y: 0,
                width: 45,
                height: 36
            }
        );
        assert!(padded_crop_rect(
            100,
            80,
            CropRect {
                x: 100,
                y: 0,
                width: 1,
                height: 1
            },
            0
        )
        .is_err());
    }

    #[test]
    fn diff_thresholding_groups_regions_and_respects_masks() {
        let baseline = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 3, Rgba([0, 0, 0, 255])));
        let mut current = baseline.to_rgba8();
        current.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        current.put_pixel(1, 0, Rgba([255, 255, 255, 255]));
        current.put_pixel(3, 2, Rgba([10, 10, 10, 255]));
        let result = compare_images(
            &baseline,
            &DynamicImage::ImageRgba8(current),
            0.1,
            &[DiffMask {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
            }],
        )
        .unwrap();
        assert_eq!(result.changed_pixels, 1);
        assert_eq!(
            result.regions,
            vec![ChangedRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                changed_pixels: 1
            }]
        );
    }

    #[test]
    fn ui_probe_expands_complete_target_viewport_theme_state_matrix() {
        let args = UiProbeArgs {
            targets: vec![
                TargetInput::Text("#one".to_string()),
                TargetInput::Text("#two".to_string()),
            ],
            viewports: vec![
                ProbeViewport {
                    width: 375,
                    height: 800,
                },
                ProbeViewport {
                    width: 1280,
                    height: 900,
                },
            ],
            themes: vec!["light".to_string(), "dark".to_string()],
            states: vec!["default".to_string(), "hover".to_string()],
            properties: Vec::new(),
        };
        let cells = expand_probe_matrix(&args).unwrap();
        assert_eq!(cells.len(), 16);
        assert_eq!(
            cells.first().unwrap(),
            &ProbeCell {
                target: 0,
                viewport: 0,
                theme: 0,
                state: 0
            }
        );
        assert_eq!(
            cells.last().unwrap(),
            &ProbeCell {
                target: 1,
                viewport: 1,
                theme: 1,
                state: 1
            }
        );
    }

    #[test]
    fn mark_ids_map_only_snapshot_nodes_with_refs_and_rects() {
        let nodes = vec![
            SnapshotNode {
                role: "button".to_string(),
                name: Some("Save".to_string()),
                reference: Some("e1".to_string()),
                geometry: Some(SnapshotBox {
                    x: 1,
                    y: 2,
                    width: 30,
                    height: 40,
                }),
            },
            SnapshotNode {
                role: "heading".to_string(),
                name: Some("Title".to_string()),
                reference: None,
                geometry: Some(SnapshotBox {
                    x: 0,
                    y: 0,
                    width: 5,
                    height: 5,
                }),
            },
        ];
        assert_eq!(
            map_snapshot_marks(&nodes),
            vec![MarkRecord {
                mark_id: 1,
                reference: "e1".to_string(),
                selector: "ref=e1".to_string(),
                role: "button".to_string(),
                name: Some("Save".to_string()),
                rect: Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 30.0,
                    height: 40.0
                }
            }]
        );
    }

    #[test]
    fn baseline_paths_are_confined_to_refact() {
        let root = Path::new("/repo");
        assert_eq!(
            baseline_path(root, "home.png").unwrap(),
            PathBuf::from("/repo/.refact/visual_baselines/home.png")
        );
        assert!(baseline_path(root, "../outside.png").is_err());
        assert!(baseline_path(root, "/tmp/outside.png").is_err());
    }

    #[tokio::test]
    async fn visual_diff_confirms_only_explicit_baseline_updates() {
        let app = crate::app_state::AppState::from_gcx(
            crate::global_context::tests::make_test_gcx().await,
        )
        .await;
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                10,
                false,
                Vec::new(),
                "chat".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ));
        let tool = ToolVisualDiff {
            config_path: "builtin".to_string(),
        };
        let confirm = tool
            .match_against_confirm_deny(
                ccx.clone(),
                &HashMap::from([("update_baseline".to_string(), json!(true))]),
            )
            .await
            .unwrap();
        assert_eq!(
            confirm.result,
            crate::tools::tools_description::MatchConfirmDenyResult::CONFIRMATION
        );
        let pass = tool
            .match_against_confirm_deny(ccx, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            pass.result,
            crate::tools::tools_description::MatchConfirmDenyResult::PASS
        );
    }

    #[test]
    fn all_design_tools_expose_tooljson_output_contracts() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(ToolUiProbe {
                config_path: "builtin".to_string(),
            }),
            Box::new(ToolMarkElements {
                config_path: "builtin".to_string(),
            }),
            Box::new(ToolContrastAudit {
                config_path: "builtin".to_string(),
            }),
            Box::new(ToolImageRegion {
                config_path: "builtin".to_string(),
            }),
            Box::new(ToolVisualDiff {
                config_path: "builtin".to_string(),
            }),
        ];
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.tool_description().name)
                .collect::<Vec<_>>(),
            vec![
                "ui_probe",
                "mark_elements",
                "contrast_audit",
                "image_region",
                "visual_diff"
            ]
        );
        assert!(ToolJson::new("ui_probe", "ok", json!({"matrix":[]}))
            .to_text()
            .contains("\"tool\": \"ui_probe\""));
    }
}
