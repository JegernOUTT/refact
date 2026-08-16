use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageEncoder, ImageFormat as CodecFormat, ImageReader};

use crate::llm_types::BaseModelRecord;

const SHRINK_TARGET_BYTES: usize = 4 * 1024 * 1024;
const SHRINK_MIN_LONGEST_SIDE: u32 = 512;
const SHRINK_SCALE_NUMERATOR: u32 = 3;
const SHRINK_SCALE_DENOMINATOR: u32 = 4;
const SHRINK_JPEG_QUALITIES: [u8; 5] = [80, 85, 70, 55, 40];

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

pub fn shrink_ladder(bytes: &[u8], mime: &str) -> Result<(Vec<u8>, String), String> {
    let source_format = codec_format_for_mime(mime)?;
    let image = ImageReader::with_format(Cursor::new(bytes), source_format)
        .decode()
        .map_err(|error| format!("Image decode failed: {error}"))?;
    let source_longest_side = image.width().max(image.height());
    let mut longest_side = source_longest_side.max(SHRINK_MIN_LONGEST_SIDE);
    let mut smallest = None;

    loop {
        let candidate = if longest_side < source_longest_side {
            resize_longest_side(&image, longest_side)
        } else {
            image.clone()
        };
        for quality in SHRINK_JPEG_QUALITIES {
            let encoded = encode_jpeg(&candidate, quality)?;
            if smallest
                .as_ref()
                .is_none_or(|current: &Vec<u8>| encoded.len() < current.len())
            {
                smallest = Some(encoded.clone());
            }
            if encoded.len() <= SHRINK_TARGET_BYTES {
                return Ok((encoded, "image/jpeg".to_string()));
            }
        }

        if longest_side <= SHRINK_MIN_LONGEST_SIDE {
            break;
        }
        longest_side = (longest_side * SHRINK_SCALE_NUMERATOR / SHRINK_SCALE_DENOMINATOR)
            .max(SHRINK_MIN_LONGEST_SIDE);
    }

    let smallest_bytes = smallest.as_ref().map(Vec::len).unwrap_or(0);
    Err(format!(
        "Image could not be resized below {SHRINK_TARGET_BYTES} bytes without reducing the longest side below {SHRINK_MIN_LONGEST_SIDE}px (smallest encoded size: {smallest_bytes} bytes)"
    ))
}

fn resize_longest_side(image: &image::DynamicImage, longest_side: u32) -> image::DynamicImage {
    let current_longest_side = image.width().max(image.height());
    if current_longest_side <= longest_side {
        return image.clone();
    }
    let scale = longest_side as f64 / current_longest_side as f64;
    let width = ((image.width() as f64 * scale).round() as u32).max(1);
    let height = ((image.height() as f64 * scale).round() as u32).max(1);
    image.resize_exact(width, height, FilterType::Lanczos3)
}

fn encode_jpeg(image: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = image.to_rgb8();
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, quality)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| format!("Image encode failed: {error}"))?;
    Ok(output)
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

    fn noisy_png(width: u32, height: u32) -> Vec<u8> {
        let mut image = image::RgbImage::new(width, height);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb([
                (x.wrapping_mul(31) ^ y.wrapping_mul(17)) as u8,
                (x.wrapping_mul(13).wrapping_add(y.wrapping_mul(29))) as u8,
                (x.wrapping_mul(7) ^ y.wrapping_mul(43)) as u8,
            ]);
        }
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut bytes), CodecFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn policy_uses_defaults_without_metadata() {
        assert_eq!(
            ImagePolicy::for_model(&BaseModelRecord::default()),
            ImagePolicy::default()
        );
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

    #[test]
    fn resize_returns_the_policy_mime() {
        let bytes = encoded_png(10, 10);
        for (format, expected) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Webp, "image/webp"),
            (ImageFormat::Jpeg, "image/jpeg"),
        ] {
            let policy = ImagePolicy {
                format,
                quality: Some(75),
                ..ImagePolicy::default()
            };
            let (encoded, mime) = resize_to_policy(&bytes, "image/png", &policy).unwrap();
            assert_eq!(mime, expected);
            assert!(image::load_from_memory(&encoded).is_ok());
        }
    }

    #[test]
    fn shrink_ladder_returns_a_smaller_supported_image() {
        let source = noisy_png(2048, 2048);
        let (shrunk, mime) = shrink_ladder(&source, "image/png").unwrap();

        assert_eq!(mime, "image/jpeg");
        assert!(shrunk.len() < source.len());
        assert!(shrunk.len() <= SHRINK_TARGET_BYTES);
        let dimensions = image::load_from_memory(&shrunk).unwrap().dimensions();
        assert!(dimensions.0.max(dimensions.1) >= SHRINK_MIN_LONGEST_SIDE);
    }

    #[test]
    fn shrink_ladder_candidates_decrease_as_dimensions_step_down() {
        let source = image::load_from_memory(&noisy_png(2048, 1536)).unwrap();
        let mut sizes = Vec::new();
        for longest_side in [2048, 1536, 1152, 864, 648, 512] {
            let candidate = resize_longest_side(&source, longest_side);
            sizes.push(encode_jpeg(&candidate, 40).unwrap().len());
        }

        assert!(sizes.windows(2).all(|pair| pair[1] < pair[0]));
    }
}
