use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use headless_chrome::protocol::cdp::Emulation;
use headless_chrome::Tab;
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use refact_browser::{
    MediaState, SnapshotBox, SnapshotNode, SnapshotOptions, ViewportState, WorldManager,
};
use refact_core::image_policy::{resize_to_policy, ImagePolicy};
use refact_integrations::browser_models::{
    BrowserElementState, BrowserLocator, BrowserScreenshotAnimations, BrowserScreenshotCaret,
    BrowserScreenshotOptions, BrowserScreenshotType, BrowserStep,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::codegraph::code_intel_api::ToolJson;
use crate::integrations::browser_controller;
use crate::integrations::browser_runtime::{find_runtime_for_chat, BrowserRuntime, ChatRuntimeLookup};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const DEFAULT_VIEWPORT_HEIGHT: u32 = 900;
const DEFAULT_DIFF_THRESHOLD: f64 = 0.1;
const BASELINE_META_SCHEMA: u32 = 1;
const BASELINE_META_SUFFIX: &str = ".meta.json";
const MAX_TARGETS: usize = 50;
const MAX_MATRIX_CELLS: usize = 400;
const MAX_AUDIT_FINDINGS: usize = 500;
const ARTIFACT_PATH_PREFIX: &str = "artifact://";
const TOKEN_FILE_NAME_MARKERS: &[&str] = &["token", "theme", "variable", "palette"];
const TOKEN_FILE_EXTENSIONS: &[&str] = &["css", "scss", "less"];
const TOKEN_DISCOVERY_SKIPPED_DIRS: &[&str] = &[
    ".git",
    ".refact",
    ".next",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
    "vendor",
];
const TOKEN_DISCOVERY_MAX_DEPTH: usize = 6;
const TOKEN_DISCOVERY_MAX_FILES: usize = 8;
const TOKEN_DISCOVERY_MAX_DIRS: usize = 2_000;
const PAGE_NOT_INSTRUMENTED_ERROR: &str = "the browser's measurement runtime (`globalThis.__refact_injected__`, installed by the `chrome` tool in an isolated world on every page it opens) is not available in the active tab, so styles, refs and element states cannot be measured right now. This happens when the page navigated or reloaded under the tool, when the browser was relaunched, or when the active tab is a blank/internal page. Re-run the `chrome` navigate step for the page and retry. Any page works, instrumentation by a build plugin is not required; the optional `refact-agent/vite-plugin-design` (`cd refact-agent/gui && npm run dev`, Vite on http://localhost:5173) only adds `data-refact-src`/`data-refact-cmp` source attribution to elements.";
const INSTRUMENTATION_ERROR_MARKERS: &[&str] = &[
    "Unknown RefactInjected method",
    "RefactInjected is not installed",
    "__refact_injected__ is not defined",
];
const ZERO_SCAN_FAILURE: &str = "contrast audit scanned 0 elements — measurement failed, not a pass; the page is not instrumented or the selector matched nothing";
const NO_TOKEN_FILES_WARNING: &str = "Contrast audit resolved 0 design-token files — non-token color findings are incomplete; results are NOT a pass";
const PROBE_ZERO_MEASUREMENT_FAILURE: &str = "ui_probe measured 0 elements — measurement failed, not a pass; the page is not instrumented or the targets matched nothing";
const AA_NORMAL_RATIO: f64 = 4.5;
const AA_LARGE_RATIO: f64 = 3.0;
const AAA_NORMAL_RATIO: f64 = 7.0;
const AAA_LARGE_RATIO: f64 = 4.5;
const NON_TEXT_RATIO: f64 = 3.0;
const MAX_INLINE_ARTIFACT_CHARS: usize = 32 * 1024;
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
pub const CONTRAST_AUDIT_EXPRESSION: &str = r#"(() => {
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
  const layerStack = (start) => {
    const layers = [];
    let cursor = start;
    while (cursor && cursor.nodeType === Node.ELEMENT_NODE) {
      const cs = getComputedStyle(cursor);
      layers.push({
        background: cs.backgroundColor,
        opacity: Number.isFinite(parseFloat(cs.opacity)) ? parseFloat(cs.opacity) : 1,
        image: !!cs.backgroundImage && cs.backgroundImage !== 'none'
      });
      cursor = cursor.parentElement;
    }
    return layers;
  };
  const rootStyle = getComputedStyle(document.documentElement);
  const canvas = (rootStyle.colorScheme || '').trim() === 'dark' ? 'dark' : 'light';
  const inactive = (el) => !!el.closest('[disabled],[aria-disabled="true"]');
  const result = [];
  const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    const text = (node.textContent || '').trim();
    const el = node.parentElement;
    if (!text || !el) continue;
    if (el.closest('script,style,noscript,template')) continue;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    if (!rect.width || !rect.height || style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) continue;
    if (inactive(el)) continue;
    result.push({kind:'text', selector: selectorFor(el), text: text.slice(0, 120), foreground: style.color, layers: layerStack(el), canvas, fontSize: parseFloat(style.fontSize) || 16, fontWeight: style.fontWeight});
  }
  for (const el of document.querySelectorAll('button,input,select,textarea,[role=button],[role=checkbox],[role=radio],[role=switch]')) {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    if (!rect.width || !rect.height || style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) continue;
    if (inactive(el)) continue;
    result.push({kind:'non_text', selector: selectorFor(el), text: '', foreground: style.borderTopColor || style.color, borderWidth: parseFloat(style.borderTopWidth) || 0, layers: layerStack(el), canvas, fontSize: 0, fontWeight: '400'});
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

pub fn is_browser_ref(value: &str) -> bool {
    value.parse::<refact_browser::Ref>().is_ok()
}

impl TargetInput {
    fn locator(&self) -> Result<BrowserLocator, String> {
        match self {
            Self::Text(value) if is_browser_ref(value) => Ok(BrowserLocator::reference(value)),
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CssColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl CssColor {
    pub const TRANSPARENT: CssColor = CssColor {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub fn opaque(self) -> Rgb {
        Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }

    fn from_rgb(rgb: Rgb, a: f64) -> Self {
        Self {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
            a: a.clamp(0.0, 1.0),
        }
    }

    fn from_linear(r: f64, g: f64, b: f64, a: f64) -> Self {
        fn encode(value: f64) -> f64 {
            let value = value.clamp(0.0, 1.0);
            let encoded = if value <= 0.003_130_8 {
                12.92 * value
            } else {
                1.055 * value.powf(1.0 / 2.4) - 0.055
            };
            (encoded * 255.0).clamp(0.0, 255.0)
        }
        Self {
            r: encode(r),
            g: encode(g),
            b: encode(b),
            a: a.clamp(0.0, 1.0),
        }
    }

    /// Source-over compositing of `self` on top of `below`, both straight (non-premultiplied).
    pub fn over(self, below: CssColor) -> CssColor {
        let alpha = self.a + below.a * (1.0 - self.a);
        if alpha <= f64::EPSILON {
            return CssColor::TRANSPARENT;
        }
        let blend =
            |top: f64, bottom: f64| (top * self.a + bottom * below.a * (1.0 - self.a)) / alpha;
        CssColor {
            r: blend(self.r, below.r),
            g: blend(self.g, below.g),
            b: blend(self.b, below.b),
            a: alpha,
        }
    }
}

/// A paint layer as seen from a text node outwards: the element's own background, the group
/// opacity it applies to everything painted inside it, and whether an image or gradient makes
/// the real backdrop unknowable from computed colors alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintLayer {
    pub background: CssColor,
    pub opacity: f64,
    pub image: bool,
}

/// Composites `innermost` (text, border or nothing) through `layers` (nearest first) onto the
/// opaque `canvas`, honouring alpha backgrounds and CSS `opacity` isolation groups.
pub fn composite_layers(layers: &[PaintLayer], innermost: Option<CssColor>, canvas: Rgb) -> Rgb {
    let mut content = innermost.unwrap_or(CssColor::TRANSPARENT);
    for layer in layers {
        content = content.over(layer.background);
        content.a *= layer.opacity.clamp(0.0, 1.0);
    }
    content.over(CssColor::from_rgb(canvas, 1.0)).opaque()
}

pub const LIGHT_CANVAS: Rgb = Rgb {
    r: 255.0,
    g: 255.0,
    b: 255.0,
};

pub const DARK_CANVAS: Rgb = Rgb {
    r: 18.0,
    g: 18.0,
    b: 18.0,
};

pub fn canvas_color(scheme: &str) -> Rgb {
    if scheme.eq_ignore_ascii_case("dark") {
        DARK_CANVAS
    } else {
        LIGHT_CANVAS
    }
}

fn parse_hex_pair(pair: &str, channel: &str) -> Result<f64, String> {
    u8::from_str_radix(pair, 16)
        .map(f64::from)
        .map_err(|_| format!("invalid {channel}"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ColorArg {
    Number(f64),
    Percent(f64),
    Angle(f64),
    None,
    Keyword,
}

impl ColorArg {
    fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }
        if let Some(percent) = raw.strip_suffix('%') {
            return percent
                .trim()
                .parse::<f64>()
                .map(Self::Percent)
                .map_err(|_| format!("invalid percentage `{raw}`"));
        }
        let lower = raw.to_ascii_lowercase();
        for (suffix, per_turn) in [
            ("deg", 360.0),
            ("grad", 400.0),
            ("rad", std::f64::consts::TAU),
            ("turn", 1.0),
        ] {
            if let Some(number) = lower.strip_suffix(suffix) {
                return number
                    .trim()
                    .parse::<f64>()
                    .map(|value| Self::Angle(value * 360.0 / per_turn))
                    .map_err(|_| format!("invalid angle `{raw}`"));
            }
        }
        if let Ok(number) = raw.parse::<f64>() {
            return Ok(Self::Number(number));
        }
        if raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
            && !raw.is_empty()
        {
            return Ok(Self::Keyword);
        }
        Err(format!("invalid color component `{raw}`"))
    }

    /// Value on a `0..=reference` scale where `100%` maps to `reference`.
    fn scaled(self, reference: f64) -> Result<f64, String> {
        match self {
            Self::Number(value) => Ok(value),
            Self::Percent(percent) => Ok(percent / 100.0 * reference),
            Self::None => Ok(0.0),
            Self::Angle(_) => Err("angle used where a number was expected".to_string()),
            Self::Keyword => Err("keyword used where a number was expected".to_string()),
        }
    }

    fn hue_degrees(self) -> Result<f64, String> {
        match self {
            Self::Number(value) | Self::Angle(value) => Ok(value.rem_euclid(360.0)),
            Self::None => Ok(0.0),
            Self::Percent(_) => Err("percentage used where a hue was expected".to_string()),
            Self::Keyword => Err("keyword used where a hue was expected".to_string()),
        }
    }

    fn alpha(self) -> Result<f64, String> {
        match self {
            Self::Number(value) => Ok(value.clamp(0.0, 1.0)),
            Self::Percent(percent) => Ok((percent / 100.0).clamp(0.0, 1.0)),
            Self::None => Ok(0.0),
            Self::Angle(_) | Self::Keyword => Err("invalid alpha".to_string()),
        }
    }
}

fn split_color_args(inner: &str) -> (Vec<&str>, Option<&str>) {
    let (components, alpha) = match inner.split_once('/') {
        Some((components, alpha)) => (components, Some(alpha.trim())),
        None => (inner, None),
    };
    if components.contains(',') {
        let mut parts = components.split(',').map(str::trim).collect::<Vec<_>>();
        if alpha.is_none() && parts.len() == 4 {
            let last = parts.pop();
            return (parts, last);
        }
        return (parts, alpha);
    }
    (components.split_whitespace().collect(), alpha)
}

fn linear_from_xyz_d65(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    (
        3.240_969_941_904_522_6 * x - 1.537_383_177_570_094 * y - 0.498_610_760_293_003_4 * z,
        -0.969_243_636_280_879_6 * x + 1.875_967_501_507_720_2 * y + 0.041_555_057_407_175_59 * z,
        0.055_630_079_696_993_66 * x - 0.203_976_958_888_976_52 * y + 1.056_971_514_242_878_6 * z,
    )
}

fn xyz_d65_from_d50(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    (
        0.955_473_452_704_218_2 * x - 0.023_098_536_874_261_423 * y + 0.063_259_308_661_021_7 * z,
        -0.028_369_706_963_208_136 * x
            + 1.009_995_458_005_822_6 * y
            + 0.021_041_398_966_943_008 * z,
        0.012_314_001_688_319_899 * x - 0.020_507_696_433_477_912 * y + 1.330_365_936_608_075_3 * z,
    )
}

fn xyz_d65_from_display_p3(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    (
        0.486_570_948_648_216_2 * r + 0.265_667_693_169_093_06 * g + 0.198_217_285_234_362_5 * b,
        0.228_974_564_069_748_8 * r + 0.691_738_521_836_506_4 * g + 0.079_286_914_093_745 * b,
        0.045_113_381_858_902_64 * g + 1.043_944_368_900_976 * b,
    )
}

fn srgb_decode(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn rgba_from_oklab(l: f64, a: f64, b: f64, alpha: f64) -> CssColor {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    CssColor::from_linear(
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3,
        alpha,
    )
}

fn rgba_from_cielab(l: f64, a: f64, b: f64, alpha: f64) -> CssColor {
    const EPSILON: f64 = 216.0 / 24389.0;
    const KAPPA: f64 = 24389.0 / 27.0;
    const WHITE_D50: (f64, f64, f64) = (0.964_22, 1.0, 0.825_21);
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;
    let xr = if fx.powi(3) > EPSILON {
        fx.powi(3)
    } else {
        (116.0 * fx - 16.0) / KAPPA
    };
    let yr = if l > KAPPA * EPSILON {
        ((l + 16.0) / 116.0).powi(3)
    } else {
        l / KAPPA
    };
    let zr = if fz.powi(3) > EPSILON {
        fz.powi(3)
    } else {
        (116.0 * fz - 16.0) / KAPPA
    };
    let (x, y, z) = xyz_d65_from_d50(xr * WHITE_D50.0, yr * WHITE_D50.1, zr * WHITE_D50.2);
    let (r, g, b) = linear_from_xyz_d65(x, y, z);
    CssColor::from_linear(r, g, b, alpha)
}

fn rgba_from_hsl(hue: f64, saturation: f64, lightness: f64, alpha: f64) -> CssColor {
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);
    let channel = |n: f64| {
        let k = (n + hue / 30.0).rem_euclid(12.0);
        let a = saturation * lightness.min(1.0 - lightness);
        lightness - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    CssColor {
        r: channel(0.0) * 255.0,
        g: channel(8.0) * 255.0,
        b: channel(4.0) * 255.0,
        a: alpha,
    }
}

fn rgba_from_hwb(hue: f64, white: f64, black: f64, alpha: f64) -> CssColor {
    let (white, black) = (white.clamp(0.0, 1.0), black.clamp(0.0, 1.0));
    if white + black >= 1.0 {
        let gray = white / (white + black) * 255.0;
        return CssColor {
            r: gray,
            g: gray,
            b: gray,
            a: alpha,
        };
    }
    let pure = rgba_from_hsl(hue, 1.0, 0.5, 1.0);
    let mix = |channel: f64| (channel / 255.0 * (1.0 - white - black) + white) * 255.0;
    CssColor {
        r: mix(pure.r),
        g: mix(pure.g),
        b: mix(pure.b),
        a: alpha,
    }
}

/// Parses every color serialization Chrome emits from `getComputedStyle`: legacy and modern
/// `rgb()`/`rgba()`, hex 3/4/6/8, `transparent`, `hsl()`, `hwb()`, `lab()`, `lch()`,
/// `oklab()`, `oklch()` and `color()` in the srgb, srgb-linear, display-p3 and xyz spaces.
pub fn parse_css_color_alpha(value: &str) -> Result<CssColor, String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("transparent") {
        return Ok(CssColor::TRANSPARENT);
    }
    if let Some(hex) = value.strip_prefix('#') {
        let expanded = match hex.len() {
            3 | 4 => hex.chars().flat_map(|ch| [ch, ch]).collect::<String>(),
            6 | 8 => hex.to_string(),
            _ => return Err(format!("unsupported hex color `{value}`")),
        };
        let alpha = if expanded.len() == 8 {
            parse_hex_pair(&expanded[6..8], "alpha")? / 255.0
        } else {
            1.0
        };
        return Ok(CssColor {
            r: parse_hex_pair(&expanded[0..2], "red")?,
            g: parse_hex_pair(&expanded[2..4], "green")?,
            b: parse_hex_pair(&expanded[4..6], "blue")?,
            a: alpha,
        });
    }
    let open = value
        .find('(')
        .ok_or_else(|| format!("unsupported color `{value}`"))?;
    let close = value
        .rfind(')')
        .ok_or_else(|| format!("unsupported color `{value}`"))?;
    let function = value[..open].trim().to_ascii_lowercase();
    let (raw_components, raw_alpha) = split_color_args(&value[open + 1..close]);
    let alpha = match raw_alpha {
        Some(raw) => ColorArg::parse(raw)?.alpha()?,
        None => 1.0,
    };
    let need = |count: usize| -> Result<Vec<ColorArg>, String> {
        if raw_components.len() < count {
            return Err(format!("unsupported color `{value}`"));
        }
        raw_components[..count]
            .iter()
            .map(|raw| ColorArg::parse(raw))
            .collect()
    };
    match function.as_str() {
        "rgb" | "rgba" => {
            let args = need(3)?;
            Ok(CssColor {
                r: args[0].scaled(255.0)?.clamp(0.0, 255.0),
                g: args[1].scaled(255.0)?.clamp(0.0, 255.0),
                b: args[2].scaled(255.0)?.clamp(0.0, 255.0),
                a: alpha,
            })
        }
        "hsl" | "hsla" => {
            let args = need(3)?;
            Ok(rgba_from_hsl(
                args[0].hue_degrees()?,
                args[1].scaled(100.0)? / 100.0,
                args[2].scaled(100.0)? / 100.0,
                alpha,
            ))
        }
        "hwb" => {
            let args = need(3)?;
            Ok(rgba_from_hwb(
                args[0].hue_degrees()?,
                args[1].scaled(100.0)? / 100.0,
                args[2].scaled(100.0)? / 100.0,
                alpha,
            ))
        }
        "lab" => {
            let args = need(3)?;
            Ok(rgba_from_cielab(
                args[0].scaled(100.0)?,
                args[1].scaled(125.0)?,
                args[2].scaled(125.0)?,
                alpha,
            ))
        }
        "lch" => {
            let args = need(3)?;
            let (l, c, h) = (
                args[0].scaled(100.0)?,
                args[1].scaled(150.0)?,
                args[2].hue_degrees()?.to_radians(),
            );
            Ok(rgba_from_cielab(l, c * h.cos(), c * h.sin(), alpha))
        }
        "oklab" => {
            let args = need(3)?;
            Ok(rgba_from_oklab(
                args[0].scaled(1.0)?,
                args[1].scaled(0.4)?,
                args[2].scaled(0.4)?,
                alpha,
            ))
        }
        "oklch" => {
            let args = need(3)?;
            let (l, c, h) = (
                args[0].scaled(1.0)?,
                args[1].scaled(0.4)?,
                args[2].hue_degrees()?.to_radians(),
            );
            Ok(rgba_from_oklab(l, c * h.cos(), c * h.sin(), alpha))
        }
        "color" => {
            let space = raw_components
                .first()
                .map(|space| space.trim().to_ascii_lowercase())
                .ok_or_else(|| format!("unsupported color `{value}`"))?;
            if raw_components.len() < 4 {
                return Err(format!("unsupported color `{value}`"));
            }
            let channels = raw_components[1..4]
                .iter()
                .map(|raw| ColorArg::parse(raw).and_then(|arg| arg.scaled(1.0)))
                .collect::<Result<Vec<_>, _>>()?;
            let (c1, c2, c3) = (channels[0], channels[1], channels[2]);
            match space.as_str() {
                "srgb" => Ok(CssColor {
                    r: (c1 * 255.0).clamp(0.0, 255.0),
                    g: (c2 * 255.0).clamp(0.0, 255.0),
                    b: (c3 * 255.0).clamp(0.0, 255.0),
                    a: alpha,
                }),
                "srgb-linear" => Ok(CssColor::from_linear(c1, c2, c3, alpha)),
                "display-p3" => {
                    let (x, y, z) =
                        xyz_d65_from_display_p3(srgb_decode(c1), srgb_decode(c2), srgb_decode(c3));
                    let (r, g, b) = linear_from_xyz_d65(x, y, z);
                    Ok(CssColor::from_linear(r, g, b, alpha))
                }
                "xyz" | "xyz-d65" => {
                    let (r, g, b) = linear_from_xyz_d65(c1, c2, c3);
                    Ok(CssColor::from_linear(r, g, b, alpha))
                }
                "xyz-d50" => {
                    let (x, y, z) = xyz_d65_from_d50(c1, c2, c3);
                    let (r, g, b) = linear_from_xyz_d65(x, y, z);
                    Ok(CssColor::from_linear(r, g, b, alpha))
                }
                other => Err(format!("unsupported color space `{other}` in `{value}`")),
            }
        }
        _ => Err(format!("unsupported color `{value}`")),
    }
}

/// Opaque color for callers that compare pixels directly; fully transparent values are an
/// error here because they have no color of their own — composite them with
/// `composite_layers` instead.
pub fn parse_css_color(value: &str) -> Result<Rgb, String> {
    let color = parse_css_color_alpha(value)?;
    if color.a <= 0.0 {
        return Err(format!("color `{value}` is fully transparent"));
    }
    Ok(color.opaque())
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

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub enum WcagLevel {
    #[default]
    #[serde(rename = "AA", alias = "aa")]
    Aa,
    #[serde(rename = "AAA", alias = "aaa")]
    Aaa,
}

impl WcagLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Aa => "AA",
            Self::Aaa => "AAA",
        }
    }
}

pub fn is_large_text(font_size: f64, font_weight: &str) -> bool {
    let weight = font_weight.parse::<u16>().unwrap_or_else(|_| {
        if font_weight.eq_ignore_ascii_case("bold") {
            700
        } else {
            400
        }
    });
    (font_size >= 18.66 && weight >= 700) || font_size >= 24.0
}

pub fn required_contrast_ratio(level: WcagLevel, large_text: bool) -> f64 {
    match (level, large_text) {
        (WcagLevel::Aa, false) => AA_NORMAL_RATIO,
        (WcagLevel::Aa, true) => AA_LARGE_RATIO,
        (WcagLevel::Aaa, false) => AAA_NORMAL_RATIO,
        (WcagLevel::Aaa, true) => AAA_LARGE_RATIO,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContrastEvaluation {
    pub ratio: f64,
    pub required_ratio: f64,
    pub large_text: bool,
    pub passes_aa: bool,
    pub passes_aaa: bool,
    pub emitted: bool,
}

pub fn evaluate_contrast(
    ratio: f64,
    level: WcagLevel,
    non_text: bool,
    font_size: f64,
    font_weight: &str,
) -> ContrastEvaluation {
    let large_text = !non_text && is_large_text(font_size, font_weight);
    let (required_ratio, passes_aa, passes_aaa) = if non_text {
        (
            NON_TEXT_RATIO,
            ratio >= NON_TEXT_RATIO,
            ratio >= NON_TEXT_RATIO,
        )
    } else {
        (
            required_contrast_ratio(level, large_text),
            ratio >= required_contrast_ratio(WcagLevel::Aa, large_text),
            ratio >= required_contrast_ratio(WcagLevel::Aaa, large_text),
        )
    };
    ContrastEvaluation {
        ratio,
        required_ratio,
        large_text,
        passes_aa,
        passes_aaa,
        emitted: ratio < required_ratio,
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CaptureViewport {
    pub width: u32,
    pub height: u32,
}

impl CaptureViewport {
    fn validate(self) -> Result<Self, String> {
        if self.width == 0 || self.height == 0 {
            return Err("viewport dimensions must be positive".to_string());
        }
        Ok(self)
    }
}

/// Geometry recorded next to a visual baseline so a later comparison can reproduce the exact
/// capture conditions instead of guessing them from whatever the session last emulated.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BaselineMeta {
    #[serde(default = "baseline_meta_schema")]
    pub schema: u32,
    pub viewport: CaptureViewport,
    pub device_scale_factor: f64,
    pub color_scheme: Option<String>,
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub captured_at: String,
}

fn baseline_meta_schema() -> u32 {
    BASELINE_META_SCHEMA
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
    #[serde(default)]
    pub viewport: Option<CaptureViewport>,
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
    #[serde(default)]
    pub level: WcagLevel,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RawContrastLayer {
    pub background: String,
    #[serde(default = "full_opacity")]
    pub opacity: f64,
    #[serde(default)]
    pub image: bool,
}

fn full_opacity() -> f64 {
    1.0
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RawContrastSample {
    pub kind: String,
    pub selector: String,
    #[serde(default)]
    pub text: String,
    pub foreground: String,
    #[serde(default)]
    pub background: String,
    #[serde(default)]
    pub layers: Vec<RawContrastLayer>,
    #[serde(default)]
    pub canvas: String,
    #[serde(default)]
    pub border_width: f64,
    #[serde(default)]
    pub font_size: f64,
    #[serde(default = "regular_weight")]
    pub font_weight: String,
}

fn regular_weight() -> String {
    "400".to_string()
}

/// What one sampled element contributes to the audit once its colors are composited.
#[derive(Clone, Debug, PartialEq)]
pub enum ContrastSampleOutcome {
    Measured {
        ratio: f64,
        foreground: Rgb,
        background: Rgb,
        boundary: &'static str,
    },
    /// An image or gradient sits in the backdrop, so computed colors cannot tell the truth.
    Unmeasurable(String),
    /// A control with neither a visible border nor a fill has no non-text boundary to test.
    Skipped,
    /// A color value the parser does not understand; reported per sample, never fatal.
    Unsupported(String),
}

fn parse_paint_layers(sample: &RawContrastSample) -> Result<Vec<PaintLayer>, String> {
    let raw = if sample.layers.is_empty() && !sample.background.is_empty() {
        vec![RawContrastLayer {
            background: sample.background.clone(),
            opacity: 1.0,
            image: false,
        }]
    } else {
        sample.layers.clone()
    };
    raw.iter()
        .map(|layer| {
            parse_css_color_alpha(&layer.background)
                .map(|background| PaintLayer {
                    background,
                    opacity: layer.opacity,
                    image: layer.image,
                })
                .map_err(|error| format!("background {error}"))
        })
        .collect()
}

/// Truncates the backdrop at the first fully opaque layer and reports an image or gradient
/// that would be visible under the element instead of inventing a ratio for it.
fn visible_backdrop(layers: &[PaintLayer]) -> Result<&[PaintLayer], String> {
    for (index, layer) in layers.iter().enumerate() {
        if layer.image {
            return Err("a background image or gradient sits under the element, so the backdrop color is not measurable from computed styles".to_string());
        }
        if layer.background.a >= 1.0 && layer.opacity >= 1.0 {
            return Ok(&layers[..=index]);
        }
    }
    Ok(layers)
}

pub fn measure_contrast_sample(sample: &RawContrastSample) -> ContrastSampleOutcome {
    let canvas = canvas_color(&sample.canvas);
    let layers = match parse_paint_layers(sample) {
        Ok(layers) => layers,
        Err(error) => return ContrastSampleOutcome::Unsupported(error),
    };
    let foreground = match parse_css_color_alpha(&sample.foreground) {
        Ok(color) => color,
        Err(error) => return ContrastSampleOutcome::Unsupported(format!("foreground {error}")),
    };
    if sample.kind == "non_text" {
        let Some((own, exterior_layers)) = layers.split_first() else {
            return ContrastSampleOutcome::Unmeasurable("no paint layers".to_string());
        };
        let exterior_layers = match visible_backdrop(exterior_layers) {
            Ok(layers) => layers,
            Err(error) => return ContrastSampleOutcome::Unmeasurable(error),
        };
        let exterior = composite_layers(exterior_layers, None, canvas);
        let border_visible = sample.border_width > 0.0 && foreground.a > 0.0;
        let (boundary_color, boundary) = if border_visible {
            (
                composite_layers(&layers, Some(foreground), canvas),
                "border",
            )
        } else if own.background.a > 0.0 {
            (composite_layers(&layers, None, canvas), "fill")
        } else {
            return ContrastSampleOutcome::Skipped;
        };
        return ContrastSampleOutcome::Measured {
            ratio: contrast_ratio(boundary_color, exterior),
            foreground: boundary_color,
            background: exterior,
            boundary,
        };
    }
    let layers = match visible_backdrop(&layers) {
        Ok(layers) => layers,
        Err(error) => return ContrastSampleOutcome::Unmeasurable(error),
    };
    let background = composite_layers(layers, None, canvas);
    let effective_foreground = composite_layers(layers, Some(foreground), canvas);
    ContrastSampleOutcome::Measured {
        ratio: contrast_ratio(effective_foreground, background),
        foreground: effective_foreground,
        background,
        boundary: "text",
    }
}

pub fn rgb_hex(color: Rgb) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        color.r.round().clamp(0.0, 255.0) as u8,
        color.g.round().clamp(0.0, 255.0) as u8,
        color.b.round().clamp(0.0, 255.0) as u8
    )
}

#[derive(Clone, Debug, Serialize)]
struct ContrastFinding {
    selector: String,
    text: String,
    foreground: String,
    background: String,
    effective_foreground: String,
    effective_background: String,
    boundary: &'static str,
    ratio: f64,
    required_ratio: f64,
    level: &'static str,
    large_text: bool,
    passes_aa: bool,
    passes_aaa: bool,
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

pub fn cell_is_measured(cell: &Value) -> bool {
    let Some(measured) = cell.get("box") else {
        return false;
    };
    ["width", "height"]
        .iter()
        .all(|key| measured.get(key).and_then(Value::as_f64).is_some())
}

pub fn artifact_is_oversized(artifact: &ImageArtifact) -> bool {
    artifact.data.len() > MAX_INLINE_ARTIFACT_CHARS
}

fn artifact_json(artifact: &ImageArtifact, path: Option<&Path>) -> Value {
    let mut value = json!({
        "kind": artifact.kind,
        "mime": artifact.mime,
        "width": artifact.width,
        "height": artifact.height,
        "bytes": artifact.bytes,
        "attached_as": "multimodal_image"
    });
    match path {
        Some(path) => {
            value["path"] = json!(path.to_string_lossy());
            value["inlined"] = json!(false);
        }
        None => {
            value["data"] = json!(artifact.data);
            value["inlined"] = json!(true);
        }
    }
    value
}

pub fn spill_artifact(
    artifact: &ImageArtifact,
    artifacts_dir: &Path,
    stem: &str,
) -> Result<Value, String> {
    if !artifact_is_oversized(artifact) {
        return Ok(artifact_json(artifact, None));
    }
    let bytes = base64::prelude::BASE64_STANDARD
        .decode(&artifact.data)
        .map_err(|error| format!("failed to decode artifact for spilling: {error}"))?;
    std::fs::create_dir_all(artifacts_dir).map_err(|error| {
        format!(
            "failed to create design artifacts directory {}: {error}",
            artifacts_dir.display()
        )
    })?;
    let extension = artifact
        .mime
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("png");
    let path = artifacts_dir.join(format!(
        "{stem}-{}.{extension}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or_default()
    ));
    std::fs::write(&path, &bytes)
        .map_err(|error| format!("failed to write artifact {}: {error}", path.display()))?;
    Ok(artifact_json(artifact, Some(&path)))
}

fn tool_image_message(
    tool_call_id: &str,
    text: String,
    artifact: &ImageArtifact,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let content = vec![
        crate::scratchpads::multimodality::MultimodalElement::new("text".to_string(), text)?,
        crate::scratchpads::multimodality::MultimodalElement::new(
            artifact.mime.clone(),
            artifact.data.clone(),
        )?,
    ];
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::Multimodal(content),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        })],
    ))
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

fn chat_runtime_lookup(ccx: &AtCommandsContext) -> ChatRuntimeLookup {
    ChatRuntimeLookup {
        chat_id: ccx.chat_id.clone(),
        root_chat_id: Some(ccx.root_chat_id.clone()).filter(|value| !value.is_empty()),
        background_agent_id: ccx.background_agent_id.clone(),
    }
}

async fn tool_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
) -> (
    crate::app_state::AppState,
    ChatRuntimeLookup,
    String,
    Option<crate::worktrees::scope::ExecutionScope>,
) {
    let ccx = ccx.lock().await;
    (
        ccx.app.clone(),
        chat_runtime_lookup(&ccx),
        ccx.current_model.clone(),
        ccx.execution_scope.clone(),
    )
}

pub fn no_runtime_error(lookup: &ChatRuntimeLookup) -> String {
    let tried = std::iter::once(lookup.chat_id.as_str())
        .chain(lookup.root_chat_id.as_deref())
        .chain(lookup.background_agent_id.as_deref())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("`, `");
    format!(
        "No browser session is open for this chat, so there is nothing to measure. \
         This is not a configuration problem: design tools attach to a browser that the \
         `chrome` tool opens. Make a `chrome` call first (for example \
         `chrome({{\"steps\":[{{\"action\":\"navigate\",\"url\":\"http://localhost:5173\"}}]}})`), \
         then retry this tool. Chat ids tried: `{tried}`."
    )
}

async fn attached_runtime(
    app: crate::app_state::AppState,
    lookup: &ChatRuntimeLookup,
) -> Result<Arc<AMutex<BrowserRuntime>>, String> {
    find_runtime_for_chat(app, lookup)
        .await
        .map(|(_, runtime)| runtime)
        .ok_or_else(|| no_runtime_error(lookup))
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
    browser_controller::resolve_locator_element(tab, world, &target.locator()?).map_err(|error| {
        map_design_runtime_error(&format!("failed to resolve {}: {error}", target.key()))
    })
}

pub fn parse_element_state(state: &str) -> Result<BrowserElementState, String> {
    match state {
        "default" => Ok(BrowserElementState::Default),
        "hover" => Ok(BrowserElementState::Hover),
        "focus" => Ok(BrowserElementState::Focus),
        "active" => Ok(BrowserElementState::Active),
        other => Err(format!(
            "unsupported state `{other}`; expected default, hover, focus, or active"
        )),
    }
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

fn find_raw_colors(
    tab: &Tab,
    world: &WorldManager,
    token_colors: &[String],
) -> Result<Vec<RawColorFinding>, String> {
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
    let value = world.eval_in_utility(tab, &expression).map_err(|error| {
        map_design_runtime_error(&format!("failed to inspect stylesheet colors: {error}"))
    })?;
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

impl ContrastAuditVerdict {
    /// Appends what could not be measured so a clean-looking count never hides unparsed
    /// colors or image backdrops.
    pub fn with_coverage(
        mut self,
        unsupported: usize,
        unmeasurable: usize,
        skipped: usize,
    ) -> Self {
        let mut notes = Vec::new();
        if unsupported > 0 {
            notes.push(format!(
                "{unsupported} sample(s) used color values the parser does not support"
            ));
        }
        if unmeasurable > 0 {
            notes.push(format!(
                "{unmeasurable} sample(s) sit on image or gradient backdrops and were not rated"
            ));
        }
        if skipped > 0 {
            notes.push(format!(
                "{skipped} control(s) had no visible border or fill boundary"
            ));
        }
        if notes.is_empty() {
            return self;
        }
        let note = notes.join("; ");
        self.summary = format!("{}. {note}", self.summary);
        if unsupported > 0 || unmeasurable > 0 {
            self.warning = Some(match self.warning.take() {
                Some(existing) => format!("{existing}; {note}"),
                None => note,
            });
        }
        self
    }
}

pub fn contrast_audit_verdict(
    elements_scanned: usize,
    level: WcagLevel,
    failures: usize,
    raw_colors: usize,
    resolved_token_files: usize,
) -> ContrastAuditVerdict {
    let measured = format!(
        "Contrast audit scanned {elements_scanned} elements at {}: {failures} contrast failures and {raw_colors} non-token colors",
        level.label()
    );
    if resolved_token_files > 0 {
        return ContrastAuditVerdict {
            summary: measured,
            warning: None,
        };
    }
    ContrastAuditVerdict {
        summary: format!("{NO_TOKEN_FILES_WARNING}. {measured}"),
        warning: Some(NO_TOKEN_FILES_WARNING.to_string()),
    }
}

fn contained_token_path(root: &Path, requested: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(requested);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let Ok(canonical) = resolved.canonicalize() else {
        return Ok(resolved);
    };
    let contained = root
        .canonicalize()
        .map(|root| canonical.starts_with(&root))
        .unwrap_or(false);
    if !contained {
        return Err(format!(
            "Design token file escapes the workspace: {requested}"
        ));
    }
    Ok(canonical)
}

fn token_colors_from_files(root: &Path, token_files: &[String]) -> Result<TokenColorScan, String> {
    let mut colors = Vec::new();
    let mut resolved_files = Vec::new();
    for relative in token_files {
        let path = contained_token_path(root, relative)?;
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
    Ok(TokenColorScan {
        colors,
        resolved_files,
    })
}

fn looks_like_token_file(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    if !TOKEN_FILE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()) {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    let stem = stem.to_ascii_lowercase();
    TOKEN_FILE_NAME_MARKERS
        .iter()
        .any(|marker| stem.contains(marker))
}

pub fn discover_design_token_files(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0_usize)]);
    let mut visited_dirs = 0_usize;
    while let Some((dir, depth)) = queue.pop_front() {
        if found.len() >= TOKEN_DISCOVERY_MAX_FILES || visited_dirs >= TOKEN_DISCOVERY_MAX_DIRS {
            break;
        }
        visited_dirs += 1;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if depth + 1 <= TOKEN_DISCOVERY_MAX_DEPTH
                    && !name.starts_with('.')
                    && !TOKEN_DISCOVERY_SKIPPED_DIRS.contains(&name)
                {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            if kind.is_file()
                && looks_like_token_file(&path)
                && found.len() < TOKEN_DISCOVERY_MAX_FILES
            {
                if let Some(relative) = path
                    .strip_prefix(root)
                    .ok()
                    .and_then(|value| value.to_str())
                {
                    found.push(relative.replace('\\', "/"));
                }
            }
        }
    }
    found.sort();
    found
}

fn design_artifacts_dir(gcx: &crate::global_context::GlobalContext) -> PathBuf {
    gcx.cache_dir.join("design_artifacts")
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

pub fn baseline_meta_path(baseline: &Path) -> PathBuf {
    let mut name = baseline
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(BASELINE_META_SUFFIX);
    baseline.with_file_name(name)
}

fn baseline_meta_unreadable(path: &Path, error: impl std::fmt::Display) -> String {
    format!(
        "baseline metadata {} could not be parsed: {error}; fix or delete it, or re-baseline \
         with update_baseline: true",
        path.display()
    )
}

/// A sidecar that exists but cannot be read pins no geometry, so it is reported instead of
/// being reduced to "no sidecar" — the latter sends the caller after a legacy baseline that
/// is not the actual problem.
async fn read_baseline_meta(path: &Path) -> Result<Option<BaselineMeta>, String> {
    let raw = match tokio::fs::read_to_string(path).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(baseline_meta_unreadable(path, error)),
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|error| baseline_meta_unreadable(path, error))
}

pub fn resolve_capture_viewport(
    requested: Option<CaptureViewport>,
    recorded: Option<CaptureViewport>,
    current: impl FnOnce() -> Result<CaptureViewport, String>,
) -> Result<CaptureViewport, String> {
    match requested.or(recorded) {
        Some(viewport) => viewport.validate(),
        None => current()?.validate(),
    }
}

fn runtime_css_viewport(runtime: &BrowserRuntime, tab: &Tab) -> Result<CaptureViewport, String> {
    if let Some(viewport) = runtime.context_state.viewport.as_ref() {
        if viewport.width > 0 && viewport.height > 0 {
            return Ok(CaptureViewport {
                width: viewport.width,
                height: viewport.height,
            });
        }
    }
    let measured = runtime
        .world_manager
        .eval_in_utility(tab, "[innerWidth, innerHeight]")
        .map_err(|error| {
            map_design_runtime_error(&format!("failed to read the page viewport: {error}"))
        })?;
    let side = |index: usize| {
        measured
            .get(index)
            .and_then(Value::as_f64)
            .map(|value| value.round() as i64)
            .filter(|value| *value > 0)
            .map(|value| value as u32)
    };
    match (side(0), side(1)) {
        (Some(width), Some(height)) => Ok(CaptureViewport { width, height }),
        _ => Err(
            "the page reported no usable viewport size; pass `viewport` to pin the capture geometry"
                .to_string(),
        ),
    }
}

/// Captures the viewport at an explicit CSS-pixel geometry with device scale factor 1 and no
/// policy downscale, restoring whatever emulation the session had on every path.
fn capture_pinned_screenshot(
    runtime: &mut BrowserRuntime,
    tab: &Tab,
    viewport: CaptureViewport,
) -> Result<ImageArtifact, String> {
    let original = runtime.context_state.viewport.clone();
    let pinned = ViewportState {
        width: viewport.width,
        height: viewport.height,
        device_scale_factor: 1.0,
        is_mobile: false,
        has_touch: false,
    };
    let captured = refact_browser::context_state::apply_viewport(tab, &pinned).and_then(|()| {
        capture_runtime_screenshot(
            runtime,
            &ImagePolicy::lossless_native(),
            default_page_text_masks(),
        )
    });
    match original.as_ref() {
        Some(viewport) => {
            let _ = refact_browser::context_state::apply_viewport(tab, viewport);
        }
        None => {
            let _ = tab.call_method(Emulation::ClearDeviceMetricsOverride(None));
        }
    }
    captured
}

pub fn baseline_geometry_conflict(
    baseline: &Path,
    recorded: CaptureViewport,
    requested: CaptureViewport,
) -> Option<String> {
    (recorded != requested).then(|| {
        format!(
            "baseline `{}` was captured at {}x{} (dpr 1); requested {}x{} — pass \
             update_baseline: true to re-baseline at the new geometry",
            baseline.display(),
            recorded.width,
            recorded.height,
            requested.width,
            requested.height
        )
    })
}

pub fn baseline_dimension_mismatch(
    baseline: &Path,
    baseline_size: (u32, u32),
    current_size: (u32, u32),
    viewport: CaptureViewport,
    meta: Option<&BaselineMeta>,
) -> String {
    let provenance = match meta {
        Some(meta) => format!(
            "the metadata sidecar records viewport {}x{} at dpr {} — pass update_baseline: true \
             to re-baseline at the new geometry",
            meta.viewport.width, meta.viewport.height, meta.device_scale_factor
        ),
        None => format!(
            "no metadata sidecar found; re-baseline with update_baseline: true (legacy baselines \
             carry no {BASELINE_META_SUFFIX} geometry record)"
        ),
    };
    format!(
        "baseline `{}` is {}x{} but the capture pinned at viewport {}x{} (dpr 1) is {}x{}; \
         {provenance}",
        baseline.display(),
        baseline_size.0,
        baseline_size.1,
        viewport.width,
        viewport.height,
        current_size.0,
        current_size.1
    )
}

pub fn artifact_reference_name(requested: &str) -> Option<String> {
    let trimmed = requested.trim();
    let trimmed = trimmed
        .strip_prefix(ARTIFACT_PATH_PREFIX)
        .unwrap_or(trimmed)
        .trim();
    let name = Path::new(trimmed).file_name()?.to_str()?;
    (!name.is_empty()).then(|| name.to_string())
}

async fn artifact_search_dirs(app: &crate::app_state::AppState) -> Vec<PathBuf> {
    let mut dirs = vec![design_artifacts_dir(&app.gcx)];
    let runtimes = app.integrations.browser_runtimes.clone();
    let runtimes = runtimes.lock().await;
    let arcs = runtimes.values().cloned().collect::<Vec<_>>();
    drop(runtimes);
    for arc in arcs {
        let runtime = arc.lock().await;
        dirs.push(runtime.artifacts_dir.join("screenshots"));
        dirs.push(runtime.artifacts_dir.clone());
        dirs.push(runtime.downloads_dir.clone());
    }
    dirs
}

async fn resolve_image_source(
    app: &crate::app_state::AppState,
    execution_scope: Option<&crate::worktrees::scope::ExecutionScope>,
    requested: &str,
) -> Result<PathBuf, String> {
    let raw = requested.trim();
    let raw = raw.strip_prefix(ARTIFACT_PATH_PREFIX).unwrap_or(raw).trim();
    if raw.is_empty() {
        return Err("image_path must not be empty".to_string());
    }
    if let Some(scope) = execution_scope {
        if let Ok(resolved) = scope.resolve_existing_path(Path::new(raw)) {
            return Ok(resolved.path);
        }
    }
    let direct = PathBuf::from(raw);
    if direct.is_file() {
        return Ok(direct);
    }
    let dirs = artifact_search_dirs(app).await;
    if let Some(name) = artifact_reference_name(raw) {
        for dir in &dirs {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(format!(
        "image `{requested}` was not found. Pass a filesystem path, or the `artifact.path` \
         value returned by a `chrome` screenshot or another design tool. Searched the given \
         path plus artifact directories: {}",
        dirs.iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn image_region_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "image_path":{"type":"string","description":"Filesystem path to an already-captured image, or the artifact path returned by a chrome screenshot or another design tool"},
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

/// Measures one matrix cell. Errors stay local to the cell so an offscreen target or a
/// missing selector cannot discard the rest of the table; the caller restores the original
/// viewport and media once the loop is done regardless of how many cells failed.
fn probe_matrix_cell(
    tab: &Tab,
    world: &WorldManager,
    viewport: &ViewportState,
    media: &MediaState,
    target: &TargetInput,
    state: BrowserElementState,
    properties_argument: &Value,
) -> Result<Value, String> {
    refact_browser::context_state::apply_viewport(tab, viewport)?;
    refact_browser::context_state::apply_media(tab, media)?;
    let handle = resolve_target_handle(tab, world, target)?;
    let mut scrolled_into_view = false;
    if state != BrowserElementState::Default {
        scrolled_into_view = world
            .call_function_on(
                tab,
                &handle,
                "function() { const r = this.getBoundingClientRect(); const inside = r.top >= 0 && r.left >= 0 && r.bottom <= innerHeight && r.right <= innerWidth; if (!inside) this.scrollIntoView({block: 'center', inline: 'center'}); return !inside; }",
                Vec::new(),
            )
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    }
    let probed =
        browser_controller::run_element_state_sequence(tab, world, &handle, &[state], |_| {
            world
                .call_function_on(
                    tab,
                    &handle,
                    PROBE_FUNCTION,
                    vec![properties_argument.clone()],
                )
                .map_err(|error| {
                    map_design_runtime_error(&format!("failed to probe {}: {error}", target.key()))
                })
        });
    let _ = world.release_handle(tab, &handle);
    let mut data = probed?
        .pop()
        .map(|(_, data)| data)
        .ok_or_else(|| format!("probe of {} produced no measurement", target.key()))?;
    if scrolled_into_view {
        data["scrolled_into_view"] = json!(true);
    }
    Ok(data)
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
        let states = args
            .states
            .iter()
            .map(|state| parse_element_state(state))
            .collect::<Result<Vec<_>, _>>()?;
        let (app, lookup, _, _) = tool_context(&ccx).await;
        let runtime = attached_runtime(app, &lookup).await?;
        let mut runtime = runtime.lock().await;
        let tab = browser_controller::session_tab(&mut runtime)?;
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
        let properties_argument =
            serde_json::to_value(&properties).map_err(|error| error.to_string())?;
        let mut table = Vec::with_capacity(cells.len());
        let mut applied_context = false;
        for cell in cells {
            let viewport = &args.viewports[cell.viewport];
            let theme = &args.themes[cell.theme];
            let state = states[cell.state];
            let target = &args.targets[cell.target];
            let mut row = json!({
                "target": target.key(),
                "viewport": {"width":viewport.width,"height":viewport.height},
                "theme": theme,
                "state": state.label(),
            });
            let viewport_state = ViewportState {
                width: viewport.width,
                height: viewport.height,
                device_scale_factor: 1.0,
                is_mobile: false,
                has_touch: false,
            };
            let media = MediaState {
                color_scheme: Some(theme.clone()),
                ..Default::default()
            };
            applied_context = true;
            match probe_matrix_cell(
                &tab,
                &runtime.world_manager,
                &viewport_state,
                &media,
                target,
                state,
                &properties_argument,
            ) {
                Ok(data) => {
                    row["box"] = data.get("box").cloned().unwrap_or(Value::Null);
                    row["styles"] = data.get("styles").cloned().unwrap_or(Value::Null);
                    row["overflow"] = data.get("overflow").cloned().unwrap_or(Value::Null);
                    if let Some(scrolled) = data.get("scrolled_into_view") {
                        row["scrolled_into_view"] = scrolled.clone();
                    }
                }
                Err(error) => {
                    row["error"] = json!(error);
                }
            }
            table.push(row);
        }
        if applied_context {
            if let Some(viewport) = original_viewport.as_ref() {
                let _ = refact_browser::context_state::apply_viewport(&tab, viewport);
            } else {
                let _ = tab.call_method(Emulation::ClearDeviceMetricsOverride(None));
            }
            let _ = refact_browser::context_state::apply_media(&tab, &original_media);
        }
        runtime.touch();
        let cells_measured = table.iter().filter(|cell| cell_is_measured(cell)).count();
        let cell_errors = table
            .iter()
            .filter_map(|cell| cell.get("error").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        if cells_measured == 0 {
            return Err(match cell_errors.first() {
                Some(error) => {
                    format!("{PROBE_ZERO_MEASUREMENT_FAILURE}; first cell error: {error}")
                }
                None => PROBE_ZERO_MEASUREMENT_FAILURE.to_string(),
            });
        }
        let summary = if cell_errors.is_empty() {
            format!(
                "Probed {} target-state combinations ({cells_measured} measured) without screenshots",
                table.len()
            )
        } else {
            format!(
                "Probed {} target-state combinations ({cells_measured} measured, {} failed — see per-cell `error`) without screenshots",
                table.len(),
                cell_errors.len()
            )
        };
        tool_message(
            tool_call_id,
            ToolJson::new(
                "ui_probe",
                summary,
                json!({
                    "matrix": table,
                    "cells_measured": cells_measured,
                    "cells_failed": cell_errors.len(),
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
            "Measure live DOM targets across a viewport × theme × state matrix. Returns computed styles, rectangles, and overflow flags without screenshots. Open the page with the `chrome` tool first: this tool measures the active tab of the browser session that call created, and works on any page (the `chrome` tool installs the `globalThis.__refact_injected__` measurement runtime itself; the optional `refact-agent/vite-plugin-design` for `cd refact-agent/gui && npm run dev` on http://localhost:5173 only adds source attribution). Themes are emulated through `prefers-color-scheme`, so an app that switches themes by class or attribute must be toggled through the `chrome` tool instead. Hover/focus/active cells scroll the target into view; each cell reports its own `error` instead of aborting the matrix, and the original viewport and media are restored afterwards. Fails closed: measuring 0 cells is a tool error rather than an empty pass.",
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
        let (app, lookup, model_id, _) = tool_context(&ccx).await;
        let policy = image_policy_for_model(app.gcx.clone(), &model_id).await;
        let runtime = attached_runtime(app, &lookup).await?;
        let mut runtime = runtime.lock().await;
        let tab = browser_controller::session_tab(&mut runtime)?;
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
                    mode: refact_browser::SnapshotMode::Ai,
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
        let spilled = spill_artifact(&artifact, &runtime.artifacts_dir, "mark-elements")?;
        runtime.touch();
        let summary = format!("Marked {} DOM-exact ARIA elements", marks.len());
        tool_image_message(
            tool_call_id,
            ToolJson::new(
                "mark_elements",
                summary,
                json!({"marks":marks,"artifact":spilled}),
            )
            .to_text(),
            &artifact,
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
        let (app, lookup, _, execution_scope) = tool_context(&ccx).await;
        let root = project_root(&app.gcx, execution_scope.as_ref())?;
        let token_files = if args.token_files.is_empty() {
            discover_design_token_files(&root)
        } else {
            args.token_files
        };
        let token_scan = token_colors_from_files(&root, &token_files)?;
        let runtime = attached_runtime(app, &lookup).await?;
        let mut runtime = runtime.lock().await;
        let tab = browser_controller::session_tab(&mut runtime)?;
        let raw = runtime
            .world_manager
            .eval_in_utility(&tab, CONTRAST_AUDIT_EXPRESSION)
            .map_err(|error| {
                map_design_runtime_error(&format!("failed to audit contrast: {error}"))
            })?;
        let samples: Vec<RawContrastSample> = serde_json::from_value(raw)
            .map_err(|error| format!("failed to parse contrast samples: {error}"))?;
        let elements_scanned = samples.len();
        if elements_scanned == 0 {
            return Err(ZERO_SCAN_FAILURE.to_string());
        }
        let mut findings = Vec::new();
        let mut unsupported = Vec::new();
        let mut unmeasurable = Vec::new();
        let mut skipped = 0usize;
        for sample in samples {
            let (ratio, foreground, background, boundary) = match measure_contrast_sample(&sample) {
                ContrastSampleOutcome::Measured {
                    ratio,
                    foreground,
                    background,
                    boundary,
                } => (ratio, foreground, background, boundary),
                ContrastSampleOutcome::Unsupported(error) => {
                    if unsupported.len() < MAX_AUDIT_FINDINGS {
                        unsupported.push(json!({"selector": sample.selector, "error": error}));
                    }
                    continue;
                }
                ContrastSampleOutcome::Unmeasurable(reason) => {
                    if unmeasurable.len() < MAX_AUDIT_FINDINGS {
                        unmeasurable.push(json!({"selector": sample.selector, "reason": reason}));
                    }
                    continue;
                }
                ContrastSampleOutcome::Skipped => {
                    skipped += 1;
                    continue;
                }
            };
            let evaluation = evaluate_contrast(
                ratio,
                args.level,
                sample.kind == "non_text",
                sample.font_size,
                &sample.font_weight,
            );
            if evaluation.emitted {
                let backdrop = sample
                    .layers
                    .first()
                    .map(|layer| layer.background.clone())
                    .unwrap_or(sample.background);
                findings.push(ContrastFinding {
                    selector: sample.selector,
                    text: sample.text,
                    foreground: sample.foreground,
                    background: backdrop,
                    effective_foreground: rgb_hex(foreground),
                    effective_background: rgb_hex(background),
                    boundary,
                    ratio: (ratio * 100.0).round() / 100.0,
                    required_ratio: evaluation.required_ratio,
                    level: args.level.label(),
                    large_text: evaluation.large_text,
                    passes_aa: evaluation.passes_aa,
                    passes_aaa: evaluation.passes_aaa,
                    threshold: evaluation.required_ratio,
                    aa: evaluation.passes_aa,
                    aaa: evaluation.passes_aaa,
                    severity: if evaluation.passes_aa {
                        "Medium"
                    } else {
                        "High"
                    },
                });
            }
            if findings.len() >= MAX_AUDIT_FINDINGS {
                break;
            }
        }
        let raw_colors = find_raw_colors(&tab, &runtime.world_manager, &token_scan.colors)?;
        runtime.touch();
        let verdict = contrast_audit_verdict(
            elements_scanned,
            args.level,
            findings.len(),
            raw_colors.len(),
            token_scan.resolved_files.len(),
        )
        .with_coverage(unsupported.len(), unmeasurable.len(), skipped);
        tool_message(
            tool_call_id,
            ToolJson::new(
                "contrast_audit",
                verdict.summary,
                json!({
                    "findings":findings,
                    "raw_colors":raw_colors,
                    "level":args.level.label(),
                    "thresholds":{
                        "aa_normal":AA_NORMAL_RATIO,
                        "aa_large":AA_LARGE_RATIO,
                        "aaa_normal":AAA_NORMAL_RATIO,
                        "aaa_large":AAA_LARGE_RATIO,
                        "non_text":NON_TEXT_RATIO
                    },
                    "token_files":token_files,
                    "token_files_resolved":token_scan.resolved_files,
                    "token_color_count":token_scan.colors.len(),
                    "elements_scanned":elements_scanned,
                    "unsupported_colors":unsupported,
                    "unmeasurable_backdrops":unmeasurable,
                    "skipped_without_boundary":skipped,
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
            "Audit live DOM contrast against the requested WCAG level: AA requires 4.5 for normal text and 3.0 for large text, AAA requires 7.0 for normal text and 4.5 for large text, and non-text UI contrast requires 3.0. Colors are composited the way the page paints them (translucent backgrounds and CSS opacity over the ancestors down to the canvas, CSS Color 4 values such as oklab/oklch/lab/lch/color() included); controls are rated on their visible boundary (border, else fill) against the surrounding backdrop, disabled controls are exempt, and samples on image or gradient backdrops or with unparseable colors are listed separately instead of being rated. A finding is emitted only when the measured ratio is below the requirement for the audited level and text size class. Open the page with the `chrome` tool first: the audit runs in the active tab of that session and works on any page (the `chrome` tool installs the `globalThis.__refact_injected__` measurement runtime itself; the optional `refact-agent/vite-plugin-design` for `cd refact-agent/gui && npm run dev` only adds source attribution). Also reports raw stylesheet colors absent from the token files; when token_files is omitted they are discovered under the workspace by name (tokens/theme/variables/palette .css/.scss/.less). Fails closed: scanning 0 elements is a tool error rather than a pass, and resolving 0 token files leads the summary with a warning.",
            json!({"type":"object","properties":{
                "token_files":{"type":"array","items":{"type":"string"},"description":"Repository-relative design-token CSS files; omitted means discover them under the workspace"},
                "level":{"type":"string","enum":["AA","AAA"],"default":"AA","description":"WCAG conformance level to audit against"}
            }}),
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
        let path = resolve_image_source(&app, execution_scope.as_ref(), &args.image_path).await?;
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
        let spilled = spill_artifact(&artifact, &design_artifacts_dir(&app.gcx), "image-region")?;
        let summary = format!(
            "Cropped {}x{} region from `{}`",
            rect.width,
            rect.height,
            path.display()
        );
        tool_image_message(
            tool_call_id,
            ToolJson::new(
                "image_region",
                summary,
                json!({"source":path,"region":rect,"artifact":spilled}),
            )
            .to_text(),
            &artifact,
        )
    }

    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            &self.config_path,
            "image_region",
            "Image Region",
            "Crop an already-captured image at native resolution with optional padding. Accepts a filesystem path or the artifact path returned by a `chrome` screenshot or another design tool. Returns a policy-processed image artifact.",
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
        let requested_viewport = args.viewport.map(CaptureViewport::validate).transpose()?;
        let (app, lookup, model_id, execution_scope) = tool_context(&ccx).await;
        let root = project_root(&app.gcx, execution_scope.as_ref())?;
        let baseline = baseline_path(&root, &args.baseline)?;
        let meta_path = baseline_meta_path(&baseline);
        let recorded = match read_baseline_meta(&meta_path).await {
            Ok(recorded) => recorded,
            Err(error) if args.update_baseline => {
                tracing::warn!(%error, "overwriting an unreadable visual baseline sidecar");
                None
            }
            Err(error) => return Err(error),
        };
        if !baseline.exists() && !args.update_baseline {
            return Err(format!(
                "Baseline `{}` does not exist; pass update_baseline: true to create it",
                baseline.display()
            ));
        }
        if !args.update_baseline {
            if let (Some(meta), Some(requested)) = (recorded.as_ref(), requested_viewport) {
                if let Some(error) = baseline_geometry_conflict(&baseline, meta.viewport, requested)
                {
                    return Err(error);
                }
            }
        }
        let policy = image_policy_for_model(app.gcx.clone(), &model_id).await;
        let runtime = attached_runtime(app, &lookup).await?;
        let mut runtime = runtime.lock().await;
        let tab = browser_controller::session_tab(&mut runtime)?;
        let viewport = resolve_capture_viewport(
            requested_viewport,
            recorded.as_ref().map(|meta| meta.viewport),
            || runtime_css_viewport(&runtime, &tab),
        )?;
        let current = capture_pinned_screenshot(&mut runtime, &tab, viewport)?;
        let current_bytes = base64::prelude::BASE64_STANDARD
            .decode(&current.data)
            .map_err(|error| format!("failed to decode current screenshot: {error}"))?;
        let url = tab.get_url();
        let capture = json!({
            "viewport":{"width":viewport.width,"height":viewport.height},
            "device_scale_factor":1.0,
            "width":current.width,
            "height":current.height,
            "url":url
        });
        if args.update_baseline {
            if let Some(parent) = baseline.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| format!("failed to create baseline directory: {error}"))?;
            }
            let existed = baseline.exists();
            let actor = crate::file_activity::current_actor();
            let guard = crate::file_activity::begin_write(
                actor.as_deref(),
                &baseline,
                if existed {
                    crate::file_activity::WriteIntent::Overwrite { authorized: true }
                } else {
                    crate::file_activity::WriteIntent::Create
                },
                "visual_diff",
            )
            .await?;
            tokio::fs::write(&baseline, &current_bytes)
                .await
                .map_err(|error| format!("failed to write baseline: {error}"))?;
            guard.commit_bytes(
                if existed {
                    crate::file_activity::WriteKind::Modified
                } else {
                    crate::file_activity::WriteKind::Created
                },
                Some(&current_bytes),
            );
            let meta = BaselineMeta {
                schema: BASELINE_META_SCHEMA,
                viewport,
                device_scale_factor: 1.0,
                color_scheme: runtime.context_state.media.color_scheme.clone(),
                url,
                width: current.width,
                height: current.height,
                captured_at: chrono::Utc::now().to_rfc3339(),
            };
            tokio::fs::write(
                &meta_path,
                serde_json::to_vec_pretty(&meta).map_err(|error| error.to_string())?,
            )
            .await
            .map_err(|error| format!("failed to write baseline metadata: {error}"))?;
            let attached = artifact_from_bytes(current_bytes.clone(), "image/png", &policy)?;
            let spilled =
                spill_artifact(&attached, &runtime.artifacts_dir, "visual-diff-baseline")?;
            runtime.touch();
            return tool_image_message(
                tool_call_id,
                ToolJson::new(
                    "visual_diff",
                    format!(
                        "Visual baseline updated at `{}` ({}x{} at viewport {}x{}, dpr 1)",
                        baseline.display(),
                        current.width,
                        current.height,
                        viewport.width,
                        viewport.height
                    ),
                    json!({"baseline":baseline,"baseline_meta_path":meta_path,"baseline_updated":true,"changed_pixels":0,"changed_percent":0.0,"regions":[],"capture":capture,"artifact":spilled}),
                )
                .to_text(),
                &attached,
            );
        }
        let baseline_bytes = tokio::fs::read(&baseline)
            .await
            .map_err(|error| format!("failed to read baseline: {error}"))?;
        let baseline_image = image::load_from_memory(&baseline_bytes)
            .map_err(|error| format!("failed to decode baseline: {error}"))?;
        let current_image = image::load_from_memory(&current_bytes)
            .map_err(|error| format!("failed to decode current screenshot: {error}"))?;
        if baseline_image.dimensions() != current_image.dimensions() {
            return Err(baseline_dimension_mismatch(
                &baseline,
                baseline_image.dimensions(),
                current_image.dimensions(),
                viewport,
                recorded.as_ref(),
            ));
        }
        let diff = compare_images(&baseline_image, &current_image, args.threshold, &args.masks)?;
        let artifact = artifact_from_bytes(encode_png(&diff.image)?, "image/png", &policy)?;
        let spilled = spill_artifact(&artifact, &runtime.artifacts_dir, "visual-diff")?;
        let summary = format!(
            "Visual diff changed {} of {} pixels ({:.3}%)",
            diff.changed_pixels, diff.total_pixels, diff.changed_percent
        );
        runtime.touch();
        tool_image_message(
            tool_call_id,
            ToolJson::new(
                "visual_diff",
                summary,
                json!({
                    "baseline":baseline,
                    "baseline_updated":false,
                    "baseline_meta":recorded,
                    "threshold":args.threshold,
                    "changed_pixels":diff.changed_pixels,
                    "total_pixels":diff.total_pixels,
                    "changed_percent":diff.changed_percent,
                    "regions":diff.regions,
                    "masks":args.masks,
                    "capture":capture,
                    "artifact":spilled
                }),
            )
            .to_text(),
            &artifact,
        )
    }

    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            &self.config_path,
            "visual_diff",
            "Visual Diff",
            "Compare the masked live-page screenshot with an explicit baseline stored under .refact/. Captures are pinned: the viewport is always emulated at explicit CSS pixels with device scale factor 1, encoded as lossless PNG with no policy downscale, and the session's own emulation is restored afterwards, so a sticky device_scale_factor from an earlier `chrome` call cannot change the geometry. Updating a baseline also writes a `<baseline>.meta.json` sidecar recording that geometry, and a comparison re-applies it, so the same baseline stays comparable across sessions. Geometry resolution order: the `viewport` argument, then the sidecar, then the session's current CSS viewport. Masks are CSS pixels of the captured viewport. Never overwrites a baseline unless update_baseline is true; a geometry or dimension mismatch is reported with both geometries and the way out instead of a bare pixel-size error. Returns pixel statistics, changed regions, the resolved `capture` geometry, the baseline sidecar, and a policy-processed diff artifact.",
            json!({
                "type":"object",
                "properties":{
                    "baseline":{"type":"string","description":"Repository-relative baseline name or .refact path"},
                    "threshold":{"type":"number","minimum":0,"maximum":1,"default":DEFAULT_DIFF_THRESHOLD},
                    "masks":{"type":"array","items":{"type":"object","properties":{"x":{"type":"integer","minimum":0},"y":{"type":"integer","minimum":0},"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1}},"required":["x","y","width","height"]},"description":"Regions ignored by the comparison, in CSS pixels of the captured viewport"},
                    "update_baseline":{"type":"boolean","default":false,"description":"Explicitly create or overwrite the baseline and its geometry sidecar"},
                    "viewport":{"type":"object","properties":{"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1}},"required":["width","height"],"description":"Pins the capture geometry in CSS pixels at device scale factor 1; omitted means the baseline sidecar, else the session's current viewport"}
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

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1.5
    }

    fn layer(background: &str, opacity: f64) -> RawContrastLayer {
        RawContrastLayer {
            background: background.to_string(),
            opacity,
            image: false,
        }
    }

    #[test]
    fn runtime_lookup_falls_back_past_the_regenerated_turn_chat_id() {
        use crate::integrations::browser_runtime::{select_runtime_for_chat, RuntimeOwner};

        let lookup = ChatRuntimeLookup {
            chat_id: "subchat-turn-2".to_string(),
            root_chat_id: Some("root-chat".to_string()),
            background_agent_id: Some("bgagent-7".to_string()),
        };
        let owned_by_root = vec![RuntimeOwner {
            runtime_id: "rt-root".to_string(),
            attached_chat_id: Some("root-chat".to_string()),
            connected: true,
        }];
        let selection = select_runtime_for_chat(&owned_by_root, &lookup)
            .expect("a stale turn id must not fail the first exact-match miss");
        assert_eq!(selection.runtime_id, "rt-root");
        assert!(selection.adopted);

        let sole_stranger = vec![RuntimeOwner {
            runtime_id: "rt-only".to_string(),
            attached_chat_id: Some("subchat-turn-1".to_string()),
            connected: true,
        }];
        assert!(
            select_runtime_for_chat(
                &sole_stranger,
                &ChatRuntimeLookup::for_chat("subchat-turn-2")
            )
            .is_none(),
            "an unrelated sole runtime must not be adopted"
        );
        let sole_unattached = vec![RuntimeOwner {
            runtime_id: "rt-only".to_string(),
            attached_chat_id: None,
            connected: true,
        }];
        assert_eq!(
            select_runtime_for_chat(&sole_unattached, &lookup)
                .unwrap()
                .runtime_id,
            "rt-only"
        );
    }

    #[test]
    fn token_files_outside_the_workspace_are_refused_instead_of_read() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.css");
        std::fs::write(&secret, ":root{--leak:#ABCDEF;}").unwrap();
        std::fs::create_dir_all(root.path().join("src/styles")).unwrap();
        std::fs::write(root.path().join("src/styles/tokens.css"), "#fff").unwrap();

        for escape in [
            secret.to_string_lossy().into_owned(),
            format!(
                "../{}/secret.css",
                outside.path().file_name().unwrap().to_string_lossy()
            ),
        ] {
            let error = token_colors_from_files(root.path(), &[escape.clone()]).unwrap_err();
            assert!(
                error.contains("escapes the workspace"),
                "{escape} -> {error}"
            );
        }

        let inside =
            token_colors_from_files(root.path(), &["src/styles/tokens.css".to_string()]).unwrap();
        assert_eq!(inside.colors, vec!["#fff"]);
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

    fn meta(width: u32, height: u32) -> BaselineMeta {
        BaselineMeta {
            schema: BASELINE_META_SCHEMA,
            viewport: CaptureViewport { width, height },
            device_scale_factor: 1.0,
            color_scheme: Some("dark".to_string()),
            url: "http://localhost:5173/".to_string(),
            width,
            height,
            captured_at: "2026-09-15T00:00:00+00:00".to_string(),
        }
    }

    #[tokio::test]
    async fn an_unreadable_baseline_sidecar_is_reported_instead_of_passing_as_absent() {
        let dir = tempfile::tempdir().unwrap();

        let missing = dir.path().join("home.png.meta.json");
        assert_eq!(read_baseline_meta(&missing).await.unwrap(), None);

        let valid = dir.path().join("valid.png.meta.json");
        tokio::fs::write(&valid, serde_json::to_vec_pretty(&meta(800, 600)).unwrap())
            .await
            .unwrap();
        assert_eq!(
            read_baseline_meta(&valid).await.unwrap(),
            Some(meta(800, 600))
        );

        for (name, body) in [
            ("garbage.png.meta.json", "{not json"),
            ("truncated.png.meta.json", "{\"viewport\":{\"width\":800}}"),
        ] {
            let path = dir.path().join(name);
            tokio::fs::write(&path, body).await.unwrap();
            let error = read_baseline_meta(&path).await.unwrap_err();
            assert!(error.contains("could not be parsed"), "{error}");
            assert!(error.contains(name), "{error}");
            assert!(error.contains("update_baseline: true"), "{error}");
            assert!(
                !error.contains("no metadata sidecar found"),
                "a sidecar that exists must never be reported as absent: {error}"
            );
        }
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
}
