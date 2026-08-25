use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use unicode_width::UnicodeWidthStr;

use crate::app::TranscriptItem;
use crate::terminal_probe::ImageProtocol;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineImage {
    pub protocol: ImageProtocol,
    pub data: Vec<u8>,
    pub mime: String,
    pub position: Position,
}

impl InlineImage {
    pub fn new(protocol: ImageProtocol, data: Vec<u8>, mime: String, position: Position) -> Self {
        Self {
            protocol,
            data,
            mime,
            position,
        }
    }
}

pub fn render_inline_images(
    writer: &mut impl Write,
    images: &[InlineImage],
    restore_position: Position,
) -> io::Result<()> {
    for image in images {
        let sequence = image_sequence(image);
        writer.write_all(cursor_position_sequence(image.position).as_bytes())?;
        writer.write_all(sequence.as_bytes())?;
    }
    if !images.is_empty() {
        writer.write_all(cursor_position_sequence(restore_position).as_bytes())?;
    }
    writer.flush()
}

pub fn write_image_to_temp_file(image: &InlineImage) -> io::Result<std::path::PathBuf> {
    let extension = image
        .mime
        .strip_prefix("image/")
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 8
                && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
        .unwrap_or("img");
    let path = std::env::temp_dir().join(format!(
        "refact-tui-image-{}-{}.{}",
        std::process::id(),
        uuid::Uuid::new_v4(),
        extension
    ));
    std::fs::write(&path, &image.data)?;
    Ok(path)
}

pub fn save_and_open_image(
    image: &InlineImage,
) -> io::Result<(std::path::PathBuf, Result<(), String>)> {
    let path = write_image_to_temp_file(image)?;
    let result = (if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&path).status()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(&path)
            .status()
    } else {
        std::process::Command::new("xdg-open").arg(&path).status()
    })
    .map_err(|error| error.to_string())
    .and_then(|status| {
        status
            .success()
            .then_some(())
            .ok_or_else(|| format!("image opener exited with {status}"))
    });
    Ok((path, result))
}

pub fn image_positions(buffer: &Buffer, items: &[TranscriptItem]) -> Vec<Position> {
    items
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::Image { placeholder, .. } => find_text_position(buffer, placeholder),
            _ => None,
        })
        .collect()
}

fn find_text_position(buffer: &Buffer, needle: &str) -> Option<Position> {
    for y in buffer.area.top()..buffer.area.bottom() {
        let mut line = String::new();
        for x in buffer.area.left()..buffer.area.right() {
            line.push_str(buffer[(x, y)].symbol());
        }
        if let Some(index) = line.find(needle) {
            return Some(Position {
                x: buffer
                    .area
                    .left()
                    .saturating_add(line[..index].width() as u16),
                y,
            });
        }
    }
    None
}

fn image_sequence(image: &InlineImage) -> String {
    match image.protocol {
        ImageProtocol::Kitty => kitty_sequence(&image.data),
        ImageProtocol::Iterm2 => iterm2_sequence(&image.data, &image.mime),
        ImageProtocol::Sixel => sixel_sequence(&image.data),
    }
}

fn cursor_position_sequence(position: Position) -> String {
    format!(
        "\x1b[{};{}H",
        position.y.saturating_add(1),
        position.x.saturating_add(1)
    )
}

fn kitty_sequence(data: &[u8]) -> String {
    let Some(png) = png_bytes(data) else {
        return String::new();
    };
    format!("\x1b_Ga=T,f=100,t=d;{}\x1b\\", STANDARD.encode(png))
}

fn iterm2_sequence(data: &[u8], mime: &str) -> String {
    format!(
        "\x1b]1337;File=inline=1;size={};type={};doNotMoveCursor=1:{}\x07",
        data.len(),
        safe_image_mime(mime),
        STANDARD.encode(data),
    )
}

fn safe_image_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "image/jpeg",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        _ => "image/png",
    }
}

fn png_bytes(data: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(data).ok()?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

fn sixel_sequence(data: &[u8]) -> String {
    let image = image::load_from_memory(data).ok();
    image
        .and_then(|image| encode_sixel(&image))
        .unwrap_or_default()
}

fn encode_sixel(image: &image::DynamicImage) -> Option<String> {
    use icy_sixel::{
        sixel_string, DiffusionMethod, MethodForLargest, MethodForRep, PixelFormat, Quality,
    };

    let rgb = image.to_rgb8();
    sixel_string(
        rgb.as_raw(),
        image.width() as i32,
        image.height() as i32,
        PixelFormat::RGB888,
        DiffusionMethod::Stucki,
        MethodForLargest::Auto,
        MethodForRep::Auto,
        Quality::HIGH,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_protocol_bytes_at_terminal_write_time() {
        let data = one_pixel_png();
        let image = InlineImage::new(
            ImageProtocol::Kitty,
            data,
            "image/png".to_string(),
            Position { x: 2, y: 3 },
        );
        let mut output = Vec::new();

        render_inline_images(&mut output, &[image], Position::ORIGIN).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("\x1b[4;3H\x1b_G"));
        assert!(output.contains("iVBOR"));
        assert!(output.ends_with("\x1b[1;1H"));
    }

    #[test]
    fn protocols_use_distinct_terminal_sequences() {
        let bytes = one_pixel_png();
        let image = |protocol| {
            InlineImage::new(
                protocol,
                bytes.clone(),
                "image/png".to_string(),
                Position::ORIGIN,
            )
        };

        assert!(image_sequence(&image(ImageProtocol::Kitty)).starts_with("\x1b_G"));
        assert!(image_sequence(&image(ImageProtocol::Iterm2)).starts_with("\x1b]1337"));
        assert!(image_sequence(&image(ImageProtocol::Sixel)).starts_with("\x1bPq"));
    }

    #[test]
    fn kitty_converts_jpeg_payloads_to_png() {
        let jpeg = image::DynamicImage::new_rgb8(1, 1);
        let mut bytes = Vec::new();
        jpeg.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
        let image = InlineImage::new(
            ImageProtocol::Kitty,
            bytes,
            "image/jpeg".to_string(),
            Position::ORIGIN,
        );

        let sequence = image_sequence(&image);

        assert!(sequence.starts_with("\x1b_Ga=T,f=100,t=d;iVBOR"));
    }

    #[test]
    fn image_positions_find_textual_fallback_without_escape_bytes() {
        let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, 40, 1));
        buffer.set_string(
            3,
            0,
            "[image: image/png, 4 bytes]",
            ratatui::style::Style::default(),
        );
        let item = TranscriptItem::Image {
            placeholder: "[image: image/png, 4 bytes]".to_string(),
            data: b"ABCD".to_vec(),
            mime: "image/png".to_string(),
        };

        assert_eq!(
            image_positions(&buffer, &[item]),
            vec![Position { x: 3, y: 0 }]
        );
        assert!(buffer
            .content()
            .iter()
            .all(|cell| !cell.symbol().contains('\x1b')));
    }

    #[test]
    fn image_positions_use_display_width_for_unicode_prefixes() {
        let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, 40, 1));
        buffer.set_string(
            0,
            0,
            "🧁 [image: image/png, 4 bytes]",
            ratatui::style::Style::default(),
        );
        let item = TranscriptItem::Image {
            placeholder: "[image: image/png, 4 bytes]".to_string(),
            data: b"ABCD".to_vec(),
            mime: "image/png".to_string(),
        };

        assert_eq!(
            image_positions(&buffer, &[item]),
            vec![Position { x: 4, y: 0 }]
        );
    }

    #[test]
    fn temp_file_uses_image_media_type_extension() {
        let image = InlineImage::new(
            ImageProtocol::Kitty,
            b"image".to_vec(),
            "image/png".to_string(),
            Position::ORIGIN,
        );

        let path = write_image_to_temp_file(&image).unwrap();

        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("png")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"image");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn temp_file_rejects_unsafe_media_type_extensions() {
        let image = InlineImage::new(
            ImageProtocol::Kitty,
            b"image".to_vec(),
            "image/png/../../escape".to_string(),
            Position::ORIGIN,
        );

        let path = write_image_to_temp_file(&image).unwrap();

        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("img")
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn iterm2_sequence_uses_a_safe_media_type() {
        let image = InlineImage::new(
            ImageProtocol::Iterm2,
            b"image".to_vec(),
            "image/png;inline=0".to_string(),
            Position::ORIGIN,
        );

        let sequence = image_sequence(&image);

        assert!(sequence.contains("type=image/png;"));
        assert!(!sequence.contains("inline=0"));
    }

    fn one_pixel_png() -> Vec<u8> {
        let image = image::DynamicImage::new_rgba8(1, 1);
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }
}
