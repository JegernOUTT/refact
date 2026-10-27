use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageDecoder, ImageEncoder, ImageFormat as CodecFormat, ImageReader};

use crate::llm_types::BaseModelRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Webp,
    Jpeg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePolicy {
    pub max_side: u32,
    pub preferred_side: u32,
    pub format: ImageFormat,
    pub quality: Option<u8>,
    pub max_images: usize,
}

impl Default for ImagePolicy {
    fn default() -> Self {
        Self {
            max_side: 2048,
            preferred_side: 1568,
            format: ImageFormat::Png,
            quality: None,
            max_images: 50,
        }
    }
}

impl ImagePolicy {
    pub fn from_metadata(max_side: Option<u32>, preferred_side: Option<u32>) -> Self {
        let mut policy = Self::default();
        if let Some(max_side) = max_side.filter(|side| *side > 0) {
            policy.max_side = max_side;
        }
        if let Some(preferred_side) = preferred_side.filter(|side| *side > 0) {
            policy.preferred_side = preferred_side.min(policy.max_side);
        } else {
            policy.preferred_side = policy.preferred_side.min(policy.max_side);
        }
        policy
    }

    pub fn for_model(model: &BaseModelRecord) -> Self {
        Self::from_metadata(model.image_max_side_px, model.image_preferred_side_px)
    }

    pub fn with_format(mut self, format: ImageFormat, quality: Option<u8>) -> Self {
        self.format = format;
        self.quality = quality;
        self
    }

    pub fn browser_capture() -> Self {
        Self {
            format: ImageFormat::Webp,
            quality: Some(80),
            ..Self::default()
        }
    }

    /// Keeps the native pixel geometry and encodes lossless PNG, for captures whose dimensions
    /// carry meaning (visual baselines) and must not be rescaled or re-encoded lossily.
    pub fn lossless_native() -> Self {
        Self {
            max_side: 0,
            preferred_side: 0,
            format: ImageFormat::Png,
            quality: None,
            ..Self::default()
        }
    }
}

pub fn resize_to_policy(
    bytes: &[u8],
    mime: &str,
    policy: &ImagePolicy,
) -> Result<(Vec<u8>, String), String> {
    let source_format = codec_format_for_mime(mime)?;
    let reader = ImageReader::with_format(Cursor::new(bytes), source_format);
    let mut image = reader
        .decode()
        .map_err(|error| format!("Image decode failed: {error}"))?;
    let max_side = policy.preferred_side.min(policy.max_side);
    let current_side = image.width().max(image.height());
    if max_side > 0 && current_side > max_side {
        let scale = max_side as f64 / current_side as f64;
        let width = ((image.width() as f64 * scale).round() as u32).max(1);
        let height = ((image.height() as f64 * scale).round() as u32).max(1);
        image = image.resize_exact(width, height, FilterType::Lanczos3);
    }

    let mut output = Vec::new();
    let output_mime = match policy.format {
        ImageFormat::Png => {
            image
                .write_to(&mut Cursor::new(&mut output), CodecFormat::Png)
                .map_err(|error| format!("Image encode failed: {error}"))?;
            "image/png"
        }
        ImageFormat::Webp => {
            image
                .write_to(&mut Cursor::new(&mut output), CodecFormat::WebP)
                .map_err(|error| format!("Image encode failed: {error}"))?;
            "image/webp"
        }
        ImageFormat::Jpeg => {
            let rgb = image.to_rgb8();
            JpegEncoder::new_with_quality(&mut output, policy.quality.unwrap_or(80))
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|error| format!("Image encode failed: {error}"))?;
            "image/jpeg"
        }
    };
    Ok((output, output_mime.to_string()))
}

pub const OUTBOUND_IMAGE_BASE64_BUDGET: usize = 5_000_000;

const BUDGET_JPEG_QUALITIES: [u8; 3] = [85, 70, 55];
const BUDGET_MAX_DOWNSCALES: usize = 16;

fn base64_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

fn encode_budget_png(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    image
        .write_with_encoder(image::codecs::png::PngEncoder::new_with_quality(
            &mut output,
            image::codecs::png::CompressionType::Best,
            image::codecs::png::FilterType::Adaptive,
        ))
        .map_err(|error| format!("PNG encode failed: {error}"))?;
    Ok(output)
}

fn encode_budget_jpeg(image: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = image.to_rgb8();
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, quality)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| format!("JPEG encode failed: {error}"))?;
    Ok(output)
}

fn has_visible_transparency(image: &image::DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel[3] < 255)
}

pub fn fit_image_base64_to_budget(
    mime: &str,
    base64_payload: &str,
    budget: usize,
) -> Result<Option<(String, String)>, String> {
    use base64::Engine;

    if base64_payload.len() <= budget {
        return Ok(None);
    }
    let context = |error: String| {
        format!(
            "cannot fit {mime} image ({} base64 bytes) into {budget} base64 bytes: {error}",
            base64_payload.len()
        )
    };
    let mut decoder = crate::chat_types::image_reader_from_b64string(base64_payload)
        .map_err(context)?
        .into_decoder()
        .map_err(|error| context(format!("decode failed: {error}")))?;
    let orientation = decoder
        .orientation()
        .map_err(|error| context(format!("orientation failed: {error}")))?;
    let mut image = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| context(format!("decode failed: {error}")))?;
    image.apply_orientation(orientation);
    let transparent = has_visible_transparency(&image);
    let lossless_source = matches!(
        mime.to_ascii_lowercase().as_str(),
        "image/png" | "image/gif"
    );
    let (width, height) = (image.width(), image.height());
    let mut scale = 1.0f64;
    for _ in 0..=BUDGET_MAX_DOWNSCALES {
        let candidate_width = ((width as f64 * scale).round() as u32).max(1);
        let candidate_height = ((height as f64 * scale).round() as u32).max(1);
        let candidate = if candidate_width == width && candidate_height == height {
            image.clone()
        } else {
            image.resize_exact(candidate_width, candidate_height, FilterType::Triangle)
        };
        if transparent || (scale == 1.0 && lossless_source) {
            let png = encode_budget_png(&candidate).map_err(context)?;
            if base64_len(png.len()) <= budget {
                let encoded = base64::engine::general_purpose::STANDARD.encode(png);
                return Ok(Some(("image/png".to_string(), encoded)));
            }
        }
        if !transparent {
            for quality in BUDGET_JPEG_QUALITIES {
                let jpeg = encode_budget_jpeg(&candidate, quality).map_err(context)?;
                if base64_len(jpeg.len()) <= budget {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(jpeg);
                    return Ok(Some(("image/jpeg".to_string(), encoded)));
                }
            }
        }
        if candidate_width == 1 && candidate_height == 1 {
            break;
        }
        scale *= 0.75;
    }
    Err(context("no re-encoding or downscale fits".to_string()))
}

fn codec_format_for_mime(mime: &str) -> Result<CodecFormat, String> {
    match mime.to_ascii_lowercase().as_str() {
        "image/png" => Ok(CodecFormat::Png),
        "image/jpeg" | "image/jpg" => Ok(CodecFormat::Jpeg),
        "image/webp" => Ok(CodecFormat::WebP),
        "image/gif" => Ok(CodecFormat::Gif),
        other => Err(format!("Unsupported image MIME type: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    fn encoded_png(width: u32, height: u32) -> Vec<u8> {
        let image = image::DynamicImage::new_rgba8(width, height);
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), CodecFormat::Png)
            .unwrap();
        bytes
    }

    fn noise_png_base64(width: u32, height: u32, alpha: bool) -> String {
        use base64::Engine;
        let mut state = 0x2545_f491u32;
        let image = image::RgbaImage::from_fn(width, height, |_, _| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let [r, g, b, a] = state.to_le_bytes();
            image::Rgba([r, g, b, if alpha { a / 2 } else { 255 }])
        });
        let image = if alpha {
            image::DynamicImage::ImageRgba8(image)
        } else {
            image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(image).to_rgb8())
        };
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), CodecFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn decode_base64(payload: &str) -> image::DynamicImage {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap();
        image::load_from_memory(&bytes).unwrap()
    }

    #[test]
    fn budget_fit_keeps_fitting_payload_and_shrinks_oversized_opaque_noise() {
        let original = noise_png_base64(240, 160, false);
        let budget = 30_000;
        assert!(original.len() > 4 * budget);
        assert_eq!(
            fit_image_base64_to_budget("image/png", &original, original.len()).unwrap(),
            None
        );

        let (mime, fitted) = fit_image_base64_to_budget("image/png", &original, budget)
            .unwrap()
            .unwrap();
        assert_eq!(mime, "image/jpeg");
        assert!(fitted.len() <= budget, "{} > {budget}", fitted.len());
        let (width, height) = decode_base64(&fitted).dimensions();
        assert!(width < 240 && height < 160);
        assert!((width as f64 / height as f64 - 1.5).abs() < 0.02);
    }

    #[test]
    fn budget_fit_preserves_transparency_and_rejects_undecodable_payload() {
        let original = noise_png_base64(160, 120, true);
        let budget = original.len() / 3;
        let (mime, fitted) = fit_image_base64_to_budget("image/png", &original, budget)
            .unwrap()
            .unwrap();
        assert_eq!(mime, "image/png");
        assert!(fitted.len() <= budget);
        let decoded = decode_base64(&fitted);
        assert!(decoded.width() < 160);
        assert!(decoded.to_rgba8().pixels().any(|pixel| pixel[3] < 255));

        let error = fit_image_base64_to_budget("image/png", &"A".repeat(400), 100).unwrap_err();
        assert!(
            error.contains("cannot fit image/png image (400 base64 bytes) into 100"),
            "{error}"
        );

        let tiny = noise_png_base64(16, 16, false);
        let error = fit_image_base64_to_budget("image/png", &tiny, 8).unwrap_err();
        assert!(
            error.ends_with("no re-encoding or downscale fits"),
            "{error}"
        );
    }

    #[test]
    fn budget_fit_applies_exif_orientation_before_reencoding() {
        use base64::Engine;

        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(48, 32, |x, _| {
            if x < 24 {
                image::Rgb([240, 10, 10])
            } else {
                image::Rgb([10, 10, 240])
            }
        }));
        let jpeg = encode_budget_jpeg(&image, 95).unwrap();
        let mut exif =
            b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        exif.resize(4096, 0);
        let mut oriented = jpeg[..2].to_vec();
        oriented.extend_from_slice(&[0xff, 0xe1]);
        oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend_from_slice(&exif);
        oriented.extend_from_slice(&jpeg[2..]);
        let payload = base64::engine::general_purpose::STANDARD.encode(oriented);
        let budget = 3000;
        assert!(payload.len() > budget);
        let mut decoder = crate::chat_types::image_reader_from_b64string(&payload)
            .unwrap()
            .into_decoder()
            .unwrap();
        assert_eq!(
            decoder.orientation().unwrap(),
            image::metadata::Orientation::Rotate90
        );

        let (mime, fitted) = fit_image_base64_to_budget("image/jpeg", &payload, budget)
            .unwrap()
            .unwrap();
        assert_eq!(mime, "image/jpeg");
        assert!(fitted.len() <= budget);
        let decoded = decode_base64(&fitted).to_rgb8();
        assert_eq!(decoded.dimensions(), (32, 48));
        let top = decoded.get_pixel(16, 8);
        let bottom = decoded.get_pixel(16, 40);
        assert!(top[0] > 200 && top[2] < 50);
        assert!(bottom[2] > 200 && bottom[0] < 50);
    }

    #[test]
    fn policy_uses_and_clamps_model_metadata() {
        let model = BaseModelRecord {
            image_max_side_px: Some(1200),
            image_preferred_side_px: Some(1600),
            ..Default::default()
        };
        let policy = ImagePolicy::for_model(&model);
        assert_eq!(policy.max_side, 1200);
        assert_eq!(policy.preferred_side, 1200);
    }

    #[test]
    fn resize_preserves_aspect_ratio_and_does_not_upscale() {
        let policy = ImagePolicy::default();
        let (small, _) = resize_to_policy(&encoded_png(320, 200), "image/png", &policy).unwrap();
        assert_eq!(
            image::load_from_memory(&small).unwrap().dimensions(),
            (320, 200)
        );

        let (large, _) = resize_to_policy(&encoded_png(4032, 3024), "image/png", &policy).unwrap();
        assert_eq!(
            image::load_from_memory(&large).unwrap().dimensions(),
            (1568, 1176)
        );
    }
}
