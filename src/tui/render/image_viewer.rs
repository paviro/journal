use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::tui::{
    image::{ImageRuntime, ImageStatus, viewer_image_size},
    state::ImageViewerState,
    theme::theme,
};

/// Draw the fullscreen image viewer. The image number is 1-based to match the
/// entry-view labels and the digit shortcut used to open it.
pub(super) fn draw_image_viewer(
    frame: &mut Frame<'_>,
    state: &ImageViewerState,
    images: &ImageRuntime,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    // The theme's background layer, not `Clear`'s terminal default.
    frame
        .buffer_mut()
        .set_style(area, super::chrome::base_style());

    let count = state.assets.len();
    let index = state.index.min(count.saturating_sub(1));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(theme().glyphs().borders.border_set())
        .border_style(theme().dialog_border())
        .title_top(Line::from(" Image Viewer "))
        .title_bottom(Line::from(format!(" Image {} of {count} ", index + 1)))
        .title_bottom(Line::from(" ←/→ navigate · esc close ").right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(asset) = state.assets.get(index) else {
        return;
    };

    if !images.enabled() {
        draw_notice(
            frame,
            inner,
            "Image display is not supported in this terminal",
        );
        return;
    }

    match images.reserve(asset, viewer_image_size(area)) {
        ImageStatus::Ready => images.render(frame, inner, asset),
        ImageStatus::Loading => draw_notice(frame, inner, &loading_notice()),
        ImageStatus::Unavailable => draw_notice(frame, inner, "Couldn't load this image"),
    }
}

/// "Loading image" notice with an ellipsis cycling `.`→`..`→`...` every ~400ms.
/// Dropped dots become spaces so the centered text doesn't jitter as it grows.
fn loading_notice() -> String {
    use std::sync::OnceLock;
    use std::time::Instant;

    static START: OnceLock<Instant> = OnceLock::new();
    let elapsed = START.get_or_init(Instant::now).elapsed();
    let dots = (elapsed.as_millis() / 400 % 3) as usize + 1;
    format!("Loading image{}{}", ".".repeat(dots), " ".repeat(3 - dots))
}

/// Center a one-line notice in `area` (used while loading or on failure).
fn draw_notice(frame: &mut Frame<'_>, area: Rect, text: &str) {
    if area.height == 0 {
        return;
    }
    let row = Rect {
        y: area.y + area.height / 2,
        height: 1,
        ..area
    };
    frame.render_widget(
        Paragraph::new(text.to_string()).alignment(Alignment::Center),
        row,
    );
}
