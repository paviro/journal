//! Image fallback rendered with a five-step, single-cell luminance ramp.

use image::GenericImageView;
use notema_storage::JournalStore;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};

use super::CacheKey;

const ASCII_DECODE_CAP: u32 = 1_600;
const LUMINANCE_RAMP: [&str; 5] = [" ", "░", "▒", "▓", "█"];
const CELL_ASPECT: f64 = 2.0;

pub(super) struct AsciiArt {
    pub(super) text: Text<'static>,
    pub(super) cols: u16,
    pub(super) rows: u16,
}

pub(super) fn build_ascii(store: &JournalStore, key: &CacheKey) -> Option<AsciiArt> {
    let bytes = store
        .read_entry_asset_bytes(&key.entry_path, &key.file_name)
        .ok()??;
    let image = notema_storage::decode_image_with_orientation(
        &bytes,
        Some((ASCII_DECODE_CAP, ASCII_DECODE_CAP)),
    )
    .ok()?;
    build_from_image(&image, key.width, key.height)
}

fn build_from_image(
    image: &image::DynamicImage,
    width_cells: u16,
    height_cells: u16,
) -> Option<AsciiArt> {
    let (image_width, image_height) = image.dimensions();
    if image_width == 0 || image_height == 0 {
        return None;
    }
    let (cols, rows) = ascii_dimensions(width_cells, height_cells, image_width, image_height);
    let resized = image
        .resize_exact(cols, rows, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let lines = resized
        .rows()
        .map(|row| {
            let spans = row
                .map(|pixel| {
                    let [red, green, blue, alpha] = pixel.0;
                    let alpha = f64::from(alpha) / 255.0;
                    let luminance = (0.2126 * f64::from(red)
                        + 0.7152 * f64::from(green)
                        + 0.0722 * f64::from(blue))
                        * alpha;
                    let index =
                        ((luminance / 255.0) * (LUMINANCE_RAMP.len() - 1) as f64).round() as usize;
                    let glyph = LUMINANCE_RAMP[index.min(LUMINANCE_RAMP.len() - 1)];
                    if alpha < 0.05 || glyph == " " {
                        Span::raw(glyph)
                    } else {
                        Span::styled(glyph, Style::new().fg(Color::Rgb(red, green, blue)))
                    }
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect::<Vec<_>>();

    Some(AsciiArt {
        text: Text::from(lines),
        cols: u16::try_from(cols).ok()?,
        rows: u16::try_from(rows).ok()?,
    })
}

fn ascii_dimensions(
    width_cells: u16,
    height_cells: u16,
    image_width: u32,
    image_height: u32,
) -> (u32, u32) {
    let available_width = f64::from(width_cells.max(1));
    let available_height = f64::from(height_cells.max(1));
    let image_aspect = f64::from(image_width) / f64::from(image_height);

    let cols = available_width;
    let rows = (cols / (image_aspect * CELL_ASPECT)).round().max(1.0);
    if rows <= available_height {
        return (cols as u32, rows as u32);
    }

    let rows = available_height;
    let cols = (rows * CELL_ASPECT * image_aspect)
        .round()
        .clamp(1.0, available_width);
    (cols as u32, rows as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic(width: u32, height: u32) -> image::DynamicImage {
        let buffer = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        image::DynamicImage::ImageRgba8(buffer)
    }

    #[test]
    fn native_fallback_survives_representative_sizes() {
        let sources = [
            synthetic(1_600, 900),
            synthetic(900, 1_600),
            synthetic(1_000, 1_000),
            synthetic(2_000, 10),
            synthetic(10, 2_000),
        ];
        let areas = [
            (1, 1),
            (1, 80),
            (200, 1),
            (2, 2),
            (10, 10),
            (37, 5),
            (5, 37),
            (80, 24),
            (200, 80),
        ];
        for source in &sources {
            for (width, height) in areas {
                assert!(build_from_image(source, width, height).is_some());
            }
        }
    }

    #[test]
    fn ramp_is_structurally_distinct_without_color() {
        let image = image::DynamicImage::ImageLuma8(
            image::GrayImage::from_vec(5, 1, vec![0, 64, 128, 192, 255]).unwrap(),
        );
        let art = build_from_image(&image, 5, 1).unwrap();
        let glyphs = art.text.lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(glyphs, " ░▒▓█");
    }
}
