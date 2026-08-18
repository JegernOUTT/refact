use std::io::Cursor;

use headless_chrome::protocol::cdp::Page;
use image::{imageops, ImageFormat as CodecFormat, Rgba, RgbaImage};
use refact_integrations::browser_models::{
    BrowserElementState, BrowserPdfOptions, BrowserScreenshotClip, BrowserScreenshotOptions,
    BrowserScreenshotScale, BrowserScreenshotType,
};

const PX_PER_INCH: f64 = 96.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenshotMetrics {
    pub page_x: f64,
    pub page_y: f64,
    pub viewport_scale: f64,
    pub device_scale_factor: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub content_width: f64,
    pub content_height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotCapture {
    pub format: Page::CaptureScreenshotFormatOption,
    pub mime: &'static str,
    pub quality: Option<u32>,
    pub clip: Page::Viewport,
    pub capture_beyond_viewport: bool,
}

pub fn screenshot_capture(
    options: &BrowserScreenshotOptions,
    metrics: ScreenshotMetrics,
    element: Option<BrowserScreenshotClip>,
) -> Result<ScreenshotCapture, String> {
    if options.full_page && element.is_some() {
        return Err("Element screenshots cannot use full_page".to_string());
    }
    if element.is_some() && options.clip.is_some() {
        return Err("Element screenshots cannot use clip".to_string());
    }
    let image_type = options.image_type.unwrap_or_default();
    let (format, mime) = match image_type {
        BrowserScreenshotType::Png => (Page::CaptureScreenshotFormatOption::Png, "image/png"),
        BrowserScreenshotType::Jpeg => (Page::CaptureScreenshotFormatOption::Jpeg, "image/jpeg"),
        BrowserScreenshotType::Webp => (Page::CaptureScreenshotFormatOption::Webp, "image/webp"),
    };
    let quality = match image_type {
        BrowserScreenshotType::Png if options.quality.is_some() => {
            return Err("quality is not supported for PNG screenshots".to_string());
        }
        BrowserScreenshotType::Jpeg => Some(options.quality.unwrap_or(80) as u32),
        BrowserScreenshotType::Webp => Some(options.quality.unwrap_or(100) as u32),
        BrowserScreenshotType::Png => None,
    };
    if quality.is_some_and(|quality| quality > 100) {
        return Err("Screenshot quality must be between 0 and 100".to_string());
    }
    let (rect, viewport_relative, capture_beyond_viewport) = if let Some(element) = element {
        (element, false, true)
    } else if options.full_page {
        let mut rect = BrowserScreenshotClip {
            x: 0.0,
            y: 0.0,
            width: metrics.content_width,
            height: metrics.content_height,
        };
        if let Some(clip) = options.clip {
            rect = intersect_clip(clip, rect)?;
        }
        (rect, false, true)
    } else if let Some(clip) = options.clip {
        let content = BrowserScreenshotClip {
            x: 0.0,
            y: 0.0,
            width: metrics.content_width,
            height: metrics.content_height,
        };
        (intersect_clip(clip, content)?, false, true)
    } else {
        (
            BrowserScreenshotClip {
                x: 0.0,
                y: 0.0,
                width: metrics.viewport_width,
                height: metrics.viewport_height,
            },
            true,
            false,
        )
    };
    let mut scale = if viewport_relative {
        metrics.viewport_scale
    } else {
        1.0
    };
    if options.scale == Some(BrowserScreenshotScale::Css) {
        scale /= metrics.device_scale_factor.max(f64::EPSILON);
    }
    Ok(ScreenshotCapture {
        format,
        mime,
        quality,
        clip: Page::Viewport {
            x: if viewport_relative {
                metrics.page_x + rect.x
            } else {
                rect.x
            },
            y: if viewport_relative {
                metrics.page_y + rect.y
            } else {
                rect.y
            },
            width: if viewport_relative {
                (rect.width / metrics.viewport_scale.max(f64::EPSILON)).ceil()
            } else {
                rect.width.ceil()
            },
            height: if viewport_relative {
                (rect.height / metrics.viewport_scale.max(f64::EPSILON)).ceil()
            } else {
                rect.height.ceil()
            },
            scale,
        },
        capture_beyond_viewport,
    })
}

fn intersect_clip(
    requested: BrowserScreenshotClip,
    bounds: BrowserScreenshotClip,
) -> Result<BrowserScreenshotClip, String> {
    if !requested.x.is_finite()
        || !requested.y.is_finite()
        || !requested.width.is_finite()
        || !requested.height.is_finite()
        || requested.width <= 0.0
        || requested.height <= 0.0
    {
        return Err("Screenshot clip must have finite positive width and height".to_string());
    }
    let x = requested.x.max(bounds.x).min(bounds.x + bounds.width);
    let y = requested.y.max(bounds.y).min(bounds.y + bounds.height);
    let right = (requested.x + requested.width)
        .max(bounds.x)
        .min(bounds.x + bounds.width);
    let bottom = (requested.y + requested.height)
        .max(bounds.y)
        .min(bounds.y + bounds.height);
    if right <= x || bottom <= y {
        return Err("Screenshot clip is outside the capture bounds".to_string());
    }
    Ok(BrowserScreenshotClip {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

pub fn pdf_payload(options: &BrowserPdfOptions) -> Result<Page::PrintToPDF, String> {
    if options.format.is_some() && (options.width.is_some() || options.height.is_some()) {
        return Err("PDF format cannot be combined with width or height".to_string());
    }
    if options
        .scale
        .is_some_and(|scale| !(0.1..=2.0).contains(&scale))
    {
        return Err("PDF scale must be between 0.1 and 2".to_string());
    }
    let (paper_width, paper_height) = if let Some(format) = options.format.as_deref() {
        paper_format(format).ok_or_else(|| format!("Unknown PDF format: {format}"))?
    } else {
        (
            parse_dimension(options.width.as_deref())?.unwrap_or(8.5),
            parse_dimension(options.height.as_deref())?.unwrap_or(11.0),
        )
    };
    let margins = options.margins.as_ref();
    Ok(Page::PrintToPDF {
        landscape: options.landscape,
        display_header_footer: Some(false),
        print_background: options.print_background,
        scale: options.scale,
        paper_width: Some(paper_width),
        paper_height: Some(paper_height),
        margin_top: parse_dimension(margins.and_then(|value| value.top.as_deref()))?,
        margin_bottom: parse_dimension(margins.and_then(|value| value.bottom.as_deref()))?,
        margin_left: parse_dimension(margins.and_then(|value| value.left.as_deref()))?,
        margin_right: parse_dimension(margins.and_then(|value| value.right.as_deref()))?,
        page_ranges: options.page_ranges.clone(),
        header_template: Some(String::new()),
        footer_template: Some(String::new()),
        prefer_css_page_size: options.prefer_css_page_size,
        transfer_mode: Some(Page::PrintToPDFTransfer_modeOption::ReturnAsStream),
        generate_tagged_pdf: options.tagged,
        generate_document_outline: options.outline,
    })
}

fn parse_dimension(value: Option<&str>) -> Result<Option<f64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    let split = value
        .find(|character: char| !character.is_ascii_digit() && character != '.' && character != '-')
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number = number
        .parse::<f64>()
        .map_err(|_| format!("Invalid PDF dimension: {value}"))?;
    if !number.is_finite() || number < 0.0 {
        return Err(format!("Invalid PDF dimension: {value}"));
    }
    let pixels = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "px" => number,
        "in" => number * PX_PER_INCH,
        "cm" => number * 37.8,
        "mm" => number * 3.78,
        other => return Err(format!("Unsupported PDF dimension unit: {other}")),
    };
    Ok(Some(pixels / PX_PER_INCH))
}

const GLYPH_WIDTH: u32 = 5;
const GLYPH_HEIGHT: u32 = 7;
const GLYPH_SCALE: u32 = 2;
const GLYPH_ADVANCE: u32 = (GLYPH_WIDTH + 1) * GLYPH_SCALE;
const LABEL_PADDING: u32 = 3;
const LABEL_BAND_HEIGHT: u32 = GLYPH_HEIGHT * GLYPH_SCALE + LABEL_PADDING * 2;
const TILE_GAP: u32 = 8;
const SHEET_PADDING: u32 = 8;
const ELLIPSIS: &str = "..";

const SHEET_BACKGROUND: Rgba<u8> = Rgba([245, 245, 245, 255]);
const TILE_BORDER: Rgba<u8> = Rgba([203, 203, 203, 255]);
const LABEL_TEXT: Rgba<u8> = Rgba([17, 17, 17, 255]);

#[rustfmt::skip]
const FONT_5X7: &[(char, [u8; 5])] = &[
    (' ', [0x00, 0x00, 0x00, 0x00, 0x00]),
    ('!', [0x00, 0x00, 0x5F, 0x00, 0x00]),
    ('"', [0x00, 0x07, 0x00, 0x07, 0x00]),
    ('#', [0x14, 0x7F, 0x14, 0x7F, 0x14]),
    ('$', [0x24, 0x2A, 0x7F, 0x2A, 0x12]),
    ('%', [0x23, 0x13, 0x08, 0x64, 0x62]),
    ('&', [0x36, 0x49, 0x55, 0x22, 0x50]),
    ('\'', [0x00, 0x05, 0x03, 0x00, 0x00]),
    ('(', [0x00, 0x1C, 0x22, 0x41, 0x00]),
    (')', [0x00, 0x41, 0x22, 0x1C, 0x00]),
    ('*', [0x14, 0x08, 0x3E, 0x08, 0x14]),
    ('+', [0x08, 0x08, 0x3E, 0x08, 0x08]),
    (',', [0x00, 0x50, 0x30, 0x00, 0x00]),
    ('-', [0x08, 0x08, 0x08, 0x08, 0x08]),
    ('.', [0x00, 0x60, 0x60, 0x00, 0x00]),
    ('/', [0x20, 0x10, 0x08, 0x04, 0x02]),
    ('0', [0x3E, 0x51, 0x49, 0x45, 0x3E]),
    ('1', [0x00, 0x42, 0x7F, 0x40, 0x00]),
    ('2', [0x42, 0x61, 0x51, 0x49, 0x46]),
    ('3', [0x21, 0x41, 0x45, 0x4B, 0x31]),
    ('4', [0x18, 0x14, 0x12, 0x7F, 0x10]),
    ('5', [0x27, 0x45, 0x45, 0x45, 0x39]),
    ('6', [0x3C, 0x4A, 0x49, 0x49, 0x30]),
    ('7', [0x01, 0x71, 0x09, 0x05, 0x03]),
    ('8', [0x36, 0x49, 0x49, 0x49, 0x36]),
    ('9', [0x06, 0x49, 0x49, 0x29, 0x1E]),
    (':', [0x00, 0x36, 0x36, 0x00, 0x00]),
    (';', [0x00, 0x56, 0x36, 0x00, 0x00]),
    ('<', [0x08, 0x14, 0x22, 0x41, 0x00]),
    ('=', [0x14, 0x14, 0x14, 0x14, 0x14]),
    ('>', [0x00, 0x41, 0x22, 0x14, 0x08]),
    ('?', [0x02, 0x01, 0x51, 0x09, 0x06]),
    ('@', [0x32, 0x49, 0x79, 0x41, 0x3E]),
    ('A', [0x7E, 0x11, 0x11, 0x11, 0x7E]),
    ('B', [0x7F, 0x49, 0x49, 0x49, 0x36]),
    ('C', [0x3E, 0x41, 0x41, 0x41, 0x22]),
    ('D', [0x7F, 0x41, 0x41, 0x22, 0x1C]),
    ('E', [0x7F, 0x49, 0x49, 0x49, 0x41]),
    ('F', [0x7F, 0x09, 0x09, 0x09, 0x01]),
    ('G', [0x3E, 0x41, 0x49, 0x49, 0x7A]),
    ('H', [0x7F, 0x08, 0x08, 0x08, 0x7F]),
    ('I', [0x00, 0x41, 0x7F, 0x41, 0x00]),
    ('J', [0x20, 0x40, 0x41, 0x3F, 0x01]),
    ('K', [0x7F, 0x08, 0x14, 0x22, 0x41]),
    ('L', [0x7F, 0x40, 0x40, 0x40, 0x40]),
    ('M', [0x7F, 0x02, 0x0C, 0x02, 0x7F]),
    ('N', [0x7F, 0x04, 0x08, 0x10, 0x7F]),
    ('O', [0x3E, 0x41, 0x41, 0x41, 0x3E]),
    ('P', [0x7F, 0x09, 0x09, 0x09, 0x06]),
    ('Q', [0x3E, 0x41, 0x51, 0x21, 0x5E]),
    ('R', [0x7F, 0x09, 0x19, 0x29, 0x46]),
    ('S', [0x46, 0x49, 0x49, 0x49, 0x31]),
    ('T', [0x01, 0x01, 0x7F, 0x01, 0x01]),
    ('U', [0x3F, 0x40, 0x40, 0x40, 0x3F]),
    ('V', [0x1F, 0x20, 0x40, 0x20, 0x1F]),
    ('W', [0x3F, 0x40, 0x38, 0x40, 0x3F]),
    ('X', [0x63, 0x14, 0x08, 0x14, 0x63]),
    ('Y', [0x07, 0x08, 0x70, 0x08, 0x07]),
    ('Z', [0x61, 0x51, 0x49, 0x45, 0x43]),
    ('[', [0x00, 0x7F, 0x41, 0x41, 0x00]),
    ('\\', [0x02, 0x04, 0x08, 0x10, 0x20]),
    (']', [0x00, 0x41, 0x41, 0x7F, 0x00]),
    ('^', [0x04, 0x02, 0x01, 0x02, 0x04]),
    ('_', [0x40, 0x40, 0x40, 0x40, 0x40]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeLayout {
    Grid,
    Strip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeTile {
    pub label: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TilePlacement {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub label_x: u32,
    pub label_y: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePlan {
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
    pub label_band: u32,
    pub placements: Vec<TilePlacement>,
}

pub fn compose_plan(
    tiles: &[ComposeTile],
    layout: ComposeLayout,
    labels: bool,
) -> Result<ComposePlan, String> {
    if tiles.is_empty() {
        return Err("Composition needs at least one capture".to_string());
    }
    if tiles.iter().any(|tile| tile.width == 0 || tile.height == 0) {
        return Err("Composition tiles must have a positive size".to_string());
    }
    let count = tiles.len() as u32;
    let columns = match layout {
        ComposeLayout::Strip => count,
        ComposeLayout::Grid => (count as f64).sqrt().ceil() as u32,
    }
    .max(1);
    let rows = count.div_ceil(columns);
    let cell_width = tiles.iter().map(|tile| tile.width).max().unwrap_or(1);
    let cell_height = tiles.iter().map(|tile| tile.height).max().unwrap_or(1);
    let label_band = if labels { LABEL_BAND_HEIGHT } else { 0 };
    let placements = tiles
        .iter()
        .enumerate()
        .map(|(index, tile)| {
            let column = index as u32 % columns;
            let row = index as u32 / columns;
            let cell_x = SHEET_PADDING + column * (cell_width + TILE_GAP);
            let cell_y = SHEET_PADDING + row * (cell_height + label_band + TILE_GAP);
            TilePlacement {
                x: cell_x + (cell_width - tile.width) / 2,
                y: cell_y + label_band + (cell_height - tile.height) / 2,
                width: tile.width,
                height: tile.height,
                label_x: cell_x,
                label_y: cell_y + LABEL_PADDING,
            }
        })
        .collect();
    Ok(ComposePlan {
        width: SHEET_PADDING * 2 + columns * cell_width + (columns - 1) * TILE_GAP,
        height: SHEET_PADDING * 2
            + rows * (cell_height + label_band)
            + rows.saturating_sub(1) * TILE_GAP,
        columns,
        rows,
        label_band,
        placements,
    })
}

pub fn compose_sheet(
    tiles: &[(String, Vec<u8>)],
    layout: ComposeLayout,
    labels: bool,
) -> Result<Vec<u8>, String> {
    let decoded = tiles
        .iter()
        .map(|(label, bytes)| {
            image::load_from_memory(bytes)
                .map(|image| (label.clone(), image.to_rgba8()))
                .map_err(|error| format!("Capture decode failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let descriptors = decoded
        .iter()
        .map(|(label, image)| ComposeTile {
            label: label.clone(),
            width: image.width(),
            height: image.height(),
        })
        .collect::<Vec<_>>();
    let plan = compose_plan(&descriptors, layout, labels)?;
    let cell_width = descriptors.iter().map(|tile| tile.width).max().unwrap_or(1);
    let mut sheet = RgbaImage::from_pixel(plan.width, plan.height, SHEET_BACKGROUND);
    for ((label, tile), placement) in decoded.iter().zip(plan.placements.iter()) {
        draw_border(&mut sheet, placement);
        imageops::overlay(&mut sheet, tile, placement.x as i64, placement.y as i64);
        if labels {
            draw_text(
                &mut sheet,
                &fit_label(label, cell_width),
                placement.label_x,
                placement.label_y,
            );
        }
    }
    let mut output = Vec::new();
    sheet
        .write_to(&mut Cursor::new(&mut output), CodecFormat::Png)
        .map_err(|error| format!("Composition encode failed: {error}"))?;
    Ok(output)
}

fn draw_border(sheet: &mut RgbaImage, placement: &TilePlacement) {
    let left = placement.x.saturating_sub(1);
    let top = placement.y.saturating_sub(1);
    let right = (placement.x + placement.width).min(sheet.width() - 1);
    let bottom = (placement.y + placement.height).min(sheet.height() - 1);
    for x in left..=right {
        sheet.put_pixel(x, top, TILE_BORDER);
        sheet.put_pixel(x, bottom, TILE_BORDER);
    }
    for y in top..=bottom {
        sheet.put_pixel(left, y, TILE_BORDER);
        sheet.put_pixel(right, y, TILE_BORDER);
    }
}

fn fit_label(label: &str, available_width: u32) -> String {
    let capacity = (available_width / GLYPH_ADVANCE) as usize;
    let normalized = label
        .chars()
        .map(|character| character.to_ascii_uppercase())
        .collect::<String>();
    if capacity == 0 {
        return String::new();
    }
    if normalized.chars().count() <= capacity {
        return normalized;
    }
    if capacity <= ELLIPSIS.len() {
        return normalized.chars().take(capacity).collect();
    }
    let kept = normalized
        .chars()
        .take(capacity - ELLIPSIS.len())
        .collect::<String>();
    format!("{kept}{ELLIPSIS}")
}

fn draw_text(sheet: &mut RgbaImage, text: &str, x: u32, y: u32) {
    for (index, character) in text.chars().enumerate() {
        let Some(glyph) = glyph_for(character) else {
            continue;
        };
        let origin_x = x + index as u32 * GLYPH_ADVANCE;
        for (column_index, column) in glyph.iter().enumerate() {
            for row in 0..GLYPH_HEIGHT {
                if column >> row & 1 == 0 {
                    continue;
                }
                let pixel_x = origin_x + column_index as u32 * GLYPH_SCALE;
                let pixel_y = y + row * GLYPH_SCALE;
                for offset_x in 0..GLYPH_SCALE {
                    for offset_y in 0..GLYPH_SCALE {
                        let target_x = pixel_x + offset_x;
                        let target_y = pixel_y + offset_y;
                        if target_x < sheet.width() && target_y < sheet.height() {
                            sheet.put_pixel(target_x, target_y, LABEL_TEXT);
                        }
                    }
                }
            }
        }
    }
}

fn glyph_for(character: char) -> Option<&'static [u8; 5]> {
    let upper = character.to_ascii_uppercase();
    FONT_5X7
        .iter()
        .find(|(candidate, _)| *candidate == upper)
        .map(|(_, glyph)| glyph)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementStateAction {
    ReleaseMouse,
    MoveMouseAway,
    Blur,
    Hover,
    Focus,
    PressAndHold,
    Capture(BrowserElementState),
}

pub const DEFAULT_ELEMENT_STATES: [BrowserElementState; 4] = [
    BrowserElementState::Default,
    BrowserElementState::Hover,
    BrowserElementState::Focus,
    BrowserElementState::Active,
];

pub fn element_state_sequence(states: &[BrowserElementState]) -> Vec<ElementStateAction> {
    let requested: &[BrowserElementState] = if states.is_empty() {
        &DEFAULT_ELEMENT_STATES
    } else {
        states
    };
    let mut unique = Vec::new();
    for state in requested {
        if !unique.contains(state) {
            unique.push(*state);
        }
    }
    let mut actions = Vec::new();
    let mut pressed = false;
    let mut hovered = false;
    let mut focused = false;
    for state in unique {
        match state {
            BrowserElementState::Default => {
                if pressed {
                    actions.push(ElementStateAction::ReleaseMouse);
                    pressed = false;
                }
                if hovered {
                    actions.push(ElementStateAction::MoveMouseAway);
                    hovered = false;
                }
                if focused {
                    actions.push(ElementStateAction::Blur);
                    focused = false;
                }
            }
            BrowserElementState::Hover => {
                if pressed {
                    actions.push(ElementStateAction::ReleaseMouse);
                    pressed = false;
                }
                if focused {
                    actions.push(ElementStateAction::Blur);
                    focused = false;
                }
                if !hovered {
                    actions.push(ElementStateAction::Hover);
                    hovered = true;
                }
            }
            BrowserElementState::Focus => {
                if pressed {
                    actions.push(ElementStateAction::ReleaseMouse);
                    pressed = false;
                }
                if hovered {
                    actions.push(ElementStateAction::MoveMouseAway);
                    hovered = false;
                }
                if !focused {
                    actions.push(ElementStateAction::Focus);
                    focused = true;
                }
            }
            BrowserElementState::Active => {
                if !hovered {
                    actions.push(ElementStateAction::Hover);
                    hovered = true;
                }
                if !pressed {
                    actions.push(ElementStateAction::PressAndHold);
                    pressed = true;
                }
                focused = true;
            }
        }
        actions.push(ElementStateAction::Capture(state));
    }
    if pressed {
        actions.push(ElementStateAction::ReleaseMouse);
    }
    if hovered {
        actions.push(ElementStateAction::MoveMouseAway);
    }
    if focused {
        actions.push(ElementStateAction::Blur);
    }
    actions
}

fn paper_format(format: &str) -> Option<(f64, f64)> {
    match format.to_ascii_lowercase().as_str() {
        "letter" => Some((8.5, 11.0)),
        "legal" => Some((8.5, 14.0)),
        "tabloid" => Some((11.0, 17.0)),
        "ledger" => Some((17.0, 11.0)),
        "a0" => Some((33.1, 46.8)),
        "a1" => Some((23.4, 33.1)),
        "a2" => Some((16.54, 23.4)),
        "a3" => Some((11.7, 16.54)),
        "a4" => Some((8.27, 11.7)),
        "a5" => Some((5.83, 8.27)),
        "a6" => Some((4.13, 5.83)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_integrations::browser_models::{
        BrowserPdfMargin, BrowserScreenshotAnimations, BrowserScreenshotCaret,
    };

    fn metrics() -> ScreenshotMetrics {
        ScreenshotMetrics {
            page_x: 10.0,
            page_y: 20.0,
            viewport_scale: 2.0,
            device_scale_factor: 2.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
            content_width: 1200.0,
            content_height: 2400.0,
        }
    }

    #[test]
    fn screenshot_options_map_every_cdp_field() {
        let options = BrowserScreenshotOptions {
            full_page: true,
            clip: Some(BrowserScreenshotClip {
                x: 5.0,
                y: 6.0,
                width: 1000.0,
                height: 2000.0,
            }),
            image_type: Some(BrowserScreenshotType::Webp),
            quality: Some(77),
            scale: Some(BrowserScreenshotScale::Css),
            omit_background: true,
            animations: Some(BrowserScreenshotAnimations::Disabled),
            caret: Some(BrowserScreenshotCaret::Initial),
            mask: Vec::new(),
            mask_color: Some("#123456".to_string()),
            style: Some("body{color:red}".to_string()),
        };
        let capture = screenshot_capture(&options, metrics(), None).unwrap();
        assert_eq!(capture.format, Page::CaptureScreenshotFormatOption::Webp);
        assert_eq!(capture.mime, "image/webp");
        assert_eq!(capture.quality, Some(77));
        assert_eq!(capture.clip.x, 5.0);
        assert_eq!(capture.clip.y, 6.0);
        assert_eq!(capture.clip.width, 1000.0);
        assert_eq!(capture.clip.height, 2000.0);
        assert_eq!(capture.clip.scale, 0.5);
        assert!(capture.capture_beyond_viewport);
    }

    #[test]
    fn document_clip_can_capture_beyond_the_viewport() {
        let capture = screenshot_capture(
            &BrowserScreenshotOptions {
                clip: Some(BrowserScreenshotClip {
                    x: 500.0,
                    y: 700.0,
                    width: 100.0,
                    height: 80.0,
                }),
                ..Default::default()
            },
            metrics(),
            None,
        )
        .unwrap();
        assert_eq!(capture.clip.x, 500.0);
        assert_eq!(capture.clip.y, 700.0);
        assert_eq!(capture.clip.width, 100.0);
        assert_eq!(capture.clip.height, 80.0);
        assert_eq!(capture.clip.scale, 1.0);
        assert!(capture.capture_beyond_viewport);
    }

    #[test]
    fn viewport_capture_adds_scroll_offset_and_respects_viewport_scale() {
        let capture =
            screenshot_capture(&BrowserScreenshotOptions::default(), metrics(), None).unwrap();
        assert_eq!(capture.clip.x, 10.0);
        assert_eq!(capture.clip.y, 20.0);
        assert_eq!(capture.clip.width, 400.0);
        assert_eq!(capture.clip.height, 300.0);
        assert_eq!(capture.clip.scale, 2.0);
        assert!(!capture.capture_beyond_viewport);
    }

    #[test]
    fn element_clip_is_document_relative_and_png_rejects_quality() {
        let element = BrowserScreenshotClip {
            x: 40.0,
            y: 50.0,
            width: 60.0,
            height: 70.0,
        };
        let capture = screenshot_capture(
            &BrowserScreenshotOptions::default(),
            metrics(),
            Some(element),
        )
        .unwrap();
        assert_eq!(capture.clip.x, 40.0);
        assert_eq!(capture.clip.y, 50.0);
        assert!(capture.capture_beyond_viewport);
        assert!(screenshot_capture(
            &BrowserScreenshotOptions {
                quality: Some(80),
                ..Default::default()
            },
            metrics(),
            None
        )
        .is_err());
        assert!(screenshot_capture(
            &BrowserScreenshotOptions {
                clip: Some(element),
                ..Default::default()
            },
            metrics(),
            Some(element)
        )
        .is_err());
        assert!(screenshot_capture(
            &BrowserScreenshotOptions {
                image_type: Some(BrowserScreenshotType::Jpeg),
                quality: Some(101),
                ..Default::default()
            },
            metrics(),
            None
        )
        .is_err());
    }

    #[test]
    fn pdf_options_map_formats_dimensions_margins_and_flags() {
        let payload = pdf_payload(&BrowserPdfOptions {
            landscape: Some(true),
            print_background: Some(true),
            scale: Some(0.8),
            format: Some("A4".to_string()),
            margins: Some(BrowserPdfMargin {
                top: Some("96px".to_string()),
                right: Some("2.54cm".to_string()),
                bottom: Some("25.4mm".to_string()),
                left: Some("1in".to_string()),
            }),
            page_ranges: Some("1-2".to_string()),
            prefer_css_page_size: Some(true),
            tagged: Some(true),
            outline: Some(true),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(payload.paper_width, Some(8.27));
        assert_eq!(payload.paper_height, Some(11.7));
        assert_eq!(payload.margin_top, Some(1.0));
        assert_eq!(payload.margin_left, Some(1.0));
        assert_eq!(payload.page_ranges.as_deref(), Some("1-2"));
        assert_eq!(payload.generate_tagged_pdf, Some(true));
        assert_eq!(payload.generate_document_outline, Some(true));
        assert_eq!(
            payload.transfer_mode,
            Some(Page::PrintToPDFTransfer_modeOption::ReturnAsStream)
        );
    }

    fn tile(label: &str, width: u32, height: u32) -> ComposeTile {
        ComposeTile {
            label: label.to_string(),
            width,
            height,
        }
    }

    fn encoded_tile(width: u32, height: u32, color: Rgba<u8>) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, color);
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), CodecFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn grid_layout_uses_square_columns_and_uniform_cells() {
        let plan = compose_plan(
            &[
                tile("a", 100, 50),
                tile("b", 60, 80),
                tile("c", 40, 40),
                tile("d", 20, 20),
                tile("e", 20, 20),
            ],
            ComposeLayout::Grid,
            true,
        )
        .unwrap();

        assert_eq!(plan.columns, 3);
        assert_eq!(plan.rows, 2);
        assert_eq!(plan.label_band, LABEL_BAND_HEIGHT);
        assert_eq!(plan.width, SHEET_PADDING * 2 + 3 * 100 + 2 * TILE_GAP);
        assert_eq!(
            plan.height,
            SHEET_PADDING * 2 + 2 * (80 + LABEL_BAND_HEIGHT) + TILE_GAP
        );
        assert_eq!(plan.placements[0].x, SHEET_PADDING);
        assert_eq!(plan.placements[0].y, SHEET_PADDING + LABEL_BAND_HEIGHT + 15);
        assert_eq!(plan.placements[1].x, SHEET_PADDING + 100 + TILE_GAP + 20);
        assert_eq!(
            plan.placements[3].y,
            SHEET_PADDING + 80 + LABEL_BAND_HEIGHT + TILE_GAP + LABEL_BAND_HEIGHT + 30
        );
    }

    #[test]
    fn strip_layout_is_one_row_and_labels_are_optional() {
        let tiles = [tile("default", 30, 40), tile("hover", 30, 40)];
        let strip = compose_plan(&tiles, ComposeLayout::Strip, true).unwrap();
        assert_eq!((strip.columns, strip.rows), (2, 1));
        assert_eq!(strip.height, SHEET_PADDING * 2 + 40 + LABEL_BAND_HEIGHT);

        let unlabelled = compose_plan(&tiles, ComposeLayout::Strip, false).unwrap();
        assert_eq!(unlabelled.label_band, 0);
        assert_eq!(unlabelled.height, SHEET_PADDING * 2 + 40);
        assert_eq!(unlabelled.placements[0].y, SHEET_PADDING);
    }

    #[test]
    fn compose_plan_rejects_empty_and_degenerate_tiles() {
        assert!(compose_plan(&[], ComposeLayout::Grid, true).is_err());
        assert!(compose_plan(&[tile("a", 0, 10)], ComposeLayout::Grid, true).is_err());
    }

    #[test]
    fn compose_sheet_draws_every_tile_and_its_label() {
        let red = Rgba([255, 0, 0, 255]);
        let blue = Rgba([0, 0, 255, 255]);
        let bytes = compose_sheet(
            &[
                ("default".to_string(), encoded_tile(40, 30, red)),
                ("hover".to_string(), encoded_tile(40, 30, blue)),
            ],
            ComposeLayout::Strip,
            true,
        )
        .unwrap();
        let sheet = image::load_from_memory(&bytes).unwrap().to_rgba8();
        let plan = compose_plan(
            &[tile("default", 40, 30), tile("hover", 40, 30)],
            ComposeLayout::Strip,
            true,
        )
        .unwrap();

        assert_eq!((sheet.width(), sheet.height()), (plan.width, plan.height));
        assert_eq!(
            *sheet.get_pixel(plan.placements[0].x, plan.placements[0].y),
            red
        );
        assert_eq!(
            *sheet.get_pixel(plan.placements[1].x, plan.placements[1].y),
            blue
        );
        let label_band_has_text = (plan.placements[0].label_x..plan.placements[0].label_x + 40)
            .any(|x| {
                (plan.placements[0].label_y
                    ..plan.placements[0].label_y + GLYPH_HEIGHT * GLYPH_SCALE)
                    .any(|y| *sheet.get_pixel(x, y) == LABEL_TEXT)
            });
        assert!(label_band_has_text);
    }

    #[test]
    fn unlabelled_composition_leaves_no_text_pixels() {
        let bytes = compose_sheet(
            &[(
                "hover".to_string(),
                encoded_tile(20, 20, Rgba([9, 9, 9, 255])),
            )],
            ComposeLayout::Grid,
            false,
        )
        .unwrap();
        let sheet = image::load_from_memory(&bytes).unwrap().to_rgba8();

        assert!(sheet.pixels().all(|pixel| *pixel != LABEL_TEXT));
    }

    #[test]
    fn labels_are_uppercased_and_truncated_to_the_cell() {
        assert_eq!(fit_label("hover", 1_000), "HOVER");
        assert_eq!(fit_label("hover", GLYPH_ADVANCE * 5), "HOVER");
        assert_eq!(fit_label("hover", GLYPH_ADVANCE * 4), "HO..");
        assert_eq!(fit_label("hover", GLYPH_ADVANCE), "H");
        assert_eq!(fit_label("hover", 0), "");
    }

    #[test]
    fn element_state_sequence_drives_and_unwinds_every_state() {
        assert_eq!(
            element_state_sequence(&[
                BrowserElementState::Default,
                BrowserElementState::Hover,
                BrowserElementState::Focus,
                BrowserElementState::Active,
            ]),
            vec![
                ElementStateAction::Capture(BrowserElementState::Default),
                ElementStateAction::Hover,
                ElementStateAction::Capture(BrowserElementState::Hover),
                ElementStateAction::MoveMouseAway,
                ElementStateAction::Focus,
                ElementStateAction::Capture(BrowserElementState::Focus),
                ElementStateAction::Hover,
                ElementStateAction::PressAndHold,
                ElementStateAction::Capture(BrowserElementState::Active),
                ElementStateAction::ReleaseMouse,
                ElementStateAction::MoveMouseAway,
                ElementStateAction::Blur,
            ]
        );
    }

    #[test]
    fn element_state_sequence_defaults_deduplicates_and_returns_to_rest() {
        assert_eq!(
            element_state_sequence(&[]),
            element_state_sequence(&DEFAULT_ELEMENT_STATES)
        );
        assert_eq!(
            element_state_sequence(&[BrowserElementState::Hover, BrowserElementState::Hover]),
            vec![
                ElementStateAction::Hover,
                ElementStateAction::Capture(BrowserElementState::Hover),
                ElementStateAction::MoveMouseAway,
            ]
        );
        assert_eq!(
            element_state_sequence(&[BrowserElementState::Active, BrowserElementState::Default]),
            vec![
                ElementStateAction::Hover,
                ElementStateAction::PressAndHold,
                ElementStateAction::Capture(BrowserElementState::Active),
                ElementStateAction::ReleaseMouse,
                ElementStateAction::MoveMouseAway,
                ElementStateAction::Blur,
                ElementStateAction::Capture(BrowserElementState::Default),
            ]
        );
    }

    #[test]
    fn pdf_rejects_conflicting_or_unknown_sizes() {
        assert!(pdf_payload(&BrowserPdfOptions {
            format: Some("A4".to_string()),
            width: Some("8in".to_string()),
            ..Default::default()
        })
        .is_err());
        assert!(pdf_payload(&BrowserPdfOptions {
            format: Some("mystery".to_string()),
            ..Default::default()
        })
        .is_err());
        assert!(pdf_payload(&BrowserPdfOptions {
            width: Some("12qu".to_string()),
            ..Default::default()
        })
        .is_err());
        assert!(pdf_payload(&BrowserPdfOptions {
            scale: Some(2.1),
            ..Default::default()
        })
        .is_err());
    }
}
