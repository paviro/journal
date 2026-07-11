//! ASCII fallback backend: render a decoded image into colored ASCII art (via
//! `rascii_art`) and parse its ANSI output into ratatui [`Text`].

use notema_storage::JournalStore;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};
use unicode_width::UnicodeWidthStr;

use super::CacheKey;

/// Longest side an image is decoded to before the ASCII fallback rescales it.
/// The art is a few hundred cells at most, so a modest cap keeps decode cheap
/// while leaving `rascii_art`'s thumbnail step plenty of detail.
const ASCII_DECODE_CAP: u32 = 1600;

/// `rascii_art`'s built-in block ramp (` ░▒▓█`), a single-width shading ladder.
/// `char_w` is derived from the charset so the sizing math adapts to it.
const ASCII_CHARSET: &[&str] = rascii_art::charsets::BLOCK;

/// A terminal cell is ~twice as tall as wide; keeps the art's aspect faithful.
const CELL_ASPECT: f64 = 2.0;

/// A finished ASCII rendering: the styled art plus its footprint (columns ×
/// rows), so the viewer can center it.
pub(super) struct AsciiArt {
    pub(super) text: Text<'static>,
    pub(super) cols: u16,
    pub(super) rows: u16,
}

/// Decrypt, decode and render an image into colored ASCII art sized to fit the
/// viewer area. `None` if any step fails.
pub(super) fn build_ascii(store: &JournalStore, key: &CacheKey) -> Option<AsciiArt> {
    let bytes = store
        .read_entry_asset_bytes(&key.entry_path, &key.file_name)
        .ok()??;
    let image = notema_storage::decode_image_with_orientation(
        &bytes,
        Some((ASCII_DECODE_CAP, ASCII_DECODE_CAP)),
    )
    .ok()?;
    let (img_w, img_h) = (image.width(), image.height());
    if img_w == 0 || img_h == 0 {
        return None;
    }

    let char_w = ASCII_CHARSET
        .iter()
        .map(|glyph| glyph.width().max(1) as u32)
        .max()
        .unwrap_or(1);
    let (cols, rows) = ascii_dimensions(key.width, key.height, img_w, img_h, char_w);

    // `rascii_art` still speaks `image` 0.24, so copy our decoded pixels across.
    let source = to_rascii_image(&image)?;
    let mut buffer = String::new();
    rascii_art::render_image_to(
        &source,
        &mut buffer,
        &rascii_art::RenderOptions::new()
            .width(cols)
            .height(rows)
            .colored(true)
            .charset(ASCII_CHARSET),
    )
    .ok()?;

    let term_cols = (cols * char_w).min(u32::from(u16::MAX)) as u16;
    let term_rows = rows.min(u32::from(u16::MAX)) as u16;
    Some(AsciiArt {
        text: ansi_to_text(&buffer),
        cols: term_cols,
        rows: term_rows,
    })
}

/// Pick the glyph grid (columns × rows) that best fills a `w_cells × h_cells`
/// area while preserving the image's aspect ratio. A glyph's on-screen shape is
/// `char_w : CELL_ASPECT`, so we keep `(cols·char_w) / (rows·CELL_ASPECT)` equal
/// to the image's width∶height, fitting to whichever dimension binds first.
fn ascii_dimensions(w_cells: u16, h_cells: u16, img_w: u32, img_h: u32, char_w: u32) -> (u32, u32) {
    let w_cells = f64::from(w_cells.max(1));
    let h_cells = f64::from(h_cells.max(1));
    let char_w = f64::from(char_w.max(1));
    let img_ar = f64::from(img_w) / f64::from(img_h);

    // Fill width first: as many glyphs as fit, then the rows that keep aspect.
    let cols = (w_cells / char_w).floor().max(1.0);
    let rows = (cols * char_w / (img_ar * CELL_ASPECT)).round().max(1.0);
    if rows <= h_cells {
        return (cols as u32, rows as u32);
    }

    // Too tall: fill height instead and derive columns.
    let rows = h_cells;
    let cols = (rows * CELL_ASPECT * img_ar / char_w)
        .round()
        .clamp(1.0, cols);
    (cols as u32, rows as u32)
}

/// Copy a decoded `image` 0.25 value into an `image` 0.24 `DynamicImage` (the
/// version `rascii_art` links) via its raw RGBA8 pixels.
fn to_rascii_image(image: &image::DynamicImage) -> Option<image_024::DynamicImage> {
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let buffer = image_024::RgbaImage::from_raw(width, height, rgba.into_raw())?;
    Some(image_024::DynamicImage::ImageRgba8(buffer))
}

/// Parse `rascii_art`'s colored output into ratatui [`Text`]. It only emits
/// truecolor foreground SGR (`\x1b[38;2;r;g;bm`) and reset (`\x1b[0m`) around
/// glyphs and `\n`; ratatui doesn't read ANSI, so translate those runs into
/// styled spans (crossterm downsamples RGB to the terminal palette as needed).
fn ansi_to_text(input: &str) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_color: Option<Color> = None;
    let mut active: Option<Color> = None;

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                // Consume the CSI sequence up to its final alphabetic byte.
                if chars.peek() == Some(&'[') {
                    chars.next();
                }
                let mut params = String::new();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                    params.push(d);
                }
                active = sgr_color(&params);
            }
            '\n' => {
                flush_run(&mut run, run_color, &mut spans);
                run_color = active;
                lines.push(Line::from(std::mem::take(&mut spans)));
            }
            _ => {
                if active != run_color {
                    flush_run(&mut run, run_color, &mut spans);
                    run_color = active;
                }
                run.push(c);
            }
        }
    }
    flush_run(&mut run, run_color, &mut spans);
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    Text::from(lines)
}

/// Push the accumulated run as one styled span, clearing the buffer.
fn flush_run(run: &mut String, color: Option<Color>, spans: &mut Vec<Span<'static>>) {
    if run.is_empty() {
        return;
    }
    let style = match color {
        Some(color) => Style::default().fg(color),
        None => Style::default(),
    };
    spans.push(Span::styled(std::mem::take(run), style));
}

/// Map an SGR parameter string to a foreground color, or `None` for a reset or
/// anything unrecognized.
fn sgr_color(params: &str) -> Option<Color> {
    let parts: Vec<&str> = params.split(';').collect();
    if parts.len() == 5 && parts[0] == "38" && parts[1] == "2" {
        let r = parts[2].parse().ok()?;
        let g = parts[3].parse().ok()?;
        let b = parts[4].parse().ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    None
}

#[cfg(test)]
mod resize_probe {
    use super::*;

    fn synthetic(w: u32, h: u32) -> image::DynamicImage {
        let buf = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        image::DynamicImage::ImageRgba8(buf)
    }

    fn build_from_image(image: &image::DynamicImage, w_cells: u16, h_cells: u16) {
        let (img_w, img_h) = (image.width(), image.height());
        let char_w = ASCII_CHARSET
            .iter()
            .map(|glyph| glyph.width().max(1) as u32)
            .max()
            .unwrap_or(1);
        let (cols, rows) = ascii_dimensions(w_cells, h_cells, img_w, img_h, char_w);
        let source = to_rascii_image(image).unwrap();
        let mut buffer = String::new();
        rascii_art::render_image_to(
            &source,
            &mut buffer,
            &rascii_art::RenderOptions::new()
                .width(cols)
                .height(rows)
                .colored(true)
                .charset(ASCII_CHARSET),
        )
        .ok();
        let _ = ansi_to_text(&buffer);
    }

    #[test]
    fn ascii_build_survives_representative_sizes() {
        // Landscape, portrait, square, and thin-strip sources.
        let sources = [
            synthetic(1600, 900),
            synthetic(900, 1600),
            synthetic(1000, 1000),
            synthetic(2000, 10),
            synthetic(10, 2000),
        ];
        // Corner cases: 1-cell edges, thin strips, typical/large viewers. The
        // point is that `ascii_dimensions` → render → `ansi_to_text` never
        // panics at extreme aspect ratios, not to enumerate every size.
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
        for src in &sources {
            for (w, h) in areas {
                build_from_image(src, w, h);
            }
        }
    }
}
