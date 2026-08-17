use headless_chrome::protocol::cdp::Page;
use refact_integrations::browser_models::{
    BrowserPdfOptions, BrowserScreenshotClip, BrowserScreenshotOptions, BrowserScreenshotScale,
    BrowserScreenshotType,
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
