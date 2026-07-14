//! Image fallback rendered with half-block glyphs in true colour.
//!
//! Each cell is the upper-half block `▀`: its foreground paints the top pixel,
//! its background the bottom pixel, so one cell carries two vertically stacked
//! pixels. Brightness comes entirely from colour — there is no luminance ramp
//! to quantise smooth regions into speckle — and vertical resolution is doubled.
//! On a grayscale/e-ink display the colours collapse to gray, reproducing the
//! image in luminance.

use image::GenericImageView;
use notema_storage::JournalStore;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};

use super::CacheKey;

const ASCII_DECODE_CAP: u32 = 1_600;
/// The upper-half block. Foreground fills the top half, background the bottom.
const HALF_BLOCK: &str = "▀";
/// A cell is roughly twice as tall as it is wide; a half-block splits it into
/// two near-square pixels, so each cell samples two image rows.
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
    // Sample two image rows per cell row so the half-block can show both.
    let resized = image
        .resize_exact(cols, rows * 2, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let lines = (0..rows)
        .map(|cell_row| {
            let top = cell_row * 2;
            let spans = (0..cols)
                .map(|x| {
                    let fg = opaque_colour(resized.get_pixel(x, top));
                    let bg = opaque_colour(resized.get_pixel(x, top + 1));
                    Span::styled(HALF_BLOCK, Style::new().fg(fg).bg(bg))
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

/// Flatten a pixel's alpha against a black background, since a cell can only
/// hold an opaque colour.
fn opaque_colour(pixel: &image::Rgba<u8>) -> Color {
    let [red, green, blue, alpha] = pixel.0;
    let alpha = f64::from(alpha) / 255.0;
    let channel = |c: u8| (f64::from(c) * alpha).round() as u8;
    Color::Rgb(channel(red), channel(green), channel(blue))
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
    fn cells_are_half_blocks_in_true_colour() {
        let colour = image::Rgba([40, 160, 90, 255]);
        let buffer = image::RgbaImage::from_pixel(6, 6, colour);
        let art = build_from_image(&image::DynamicImage::ImageRgba8(buffer), 6, 6).unwrap();
        for line in &art.text.lines {
            for span in &line.spans {
                assert_eq!(span.content.as_ref(), HALF_BLOCK);
                // A uniform image paints the true colour into both halves.
                assert_eq!(span.style.fg, Some(Color::Rgb(40, 160, 90)));
                assert_eq!(span.style.bg, Some(Color::Rgb(40, 160, 90)));
            }
        }
    }

    #[test]
    fn half_block_carries_two_rows_per_cell() {
        // Top half red, bottom half blue: each cell's fg is the upper pixel and
        // its bg the lower, and the doubled sampling preserves the split.
        let buffer = image::RgbaImage::from_fn(8, 8, |_, y| {
            if y < 4 {
                image::Rgba([200, 20, 20, 255])
            } else {
                image::Rgba([20, 20, 200, 255])
            }
        });
        let art = build_from_image(&image::DynamicImage::ImageRgba8(buffer), 8, 8).unwrap();
        assert_eq!(art.rows, 4);
        let top = art.text.lines[0].spans[0].style.fg.unwrap();
        let bottom = art.text.lines[art.rows as usize - 1].spans[0]
            .style
            .fg
            .unwrap();
        assert!(matches!(top, Color::Rgb(r, _, b) if r > b));
        assert!(matches!(bottom, Color::Rgb(r, _, b) if b > r));
    }
}
