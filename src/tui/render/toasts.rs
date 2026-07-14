//! Toast notifications: card layout in the top-right stack, per-variant
//! styling, and click hit-testing.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::app::App;
use crate::tui::state::ToastVariant;
use crate::tui::surface::point_in_rect;
use crate::tui::theme::theme;

use super::chrome::{clear_surface, flat_chrome};
use super::footer::clamp_u16;

/// Widest a toast gets; narrower terminals shrink it further.
const TOAST_MAX_WIDTH: u16 = 44;

/// Longest a toast message renders before ellipsizing.
const TOAST_MAX_LINES: usize = 4;

/// Blank columns kept between a toast and the terminal's right edge.
const TOAST_RIGHT_INSET: u16 = 2;

/// The width of a toast card given the terminal width — capped at
/// [`TOAST_MAX_WIDTH`], shrinking on narrow terminals.
fn card_width(area_width: u16) -> u16 {
    TOAST_MAX_WIDTH.min(area_width.saturating_sub(6))
}

/// The number of columns the countdown line spans — the inner width between the
/// one-column side insets. `0` when no toast fits, so the event loop knows there
/// is no step to schedule a wake for.
pub(crate) fn countdown_cols(area_width: u16) -> u16 {
    card_width(area_width).saturating_sub(4)
}

fn toast_style(variant: ToastVariant) -> Style {
    match variant {
        ToastVariant::Info => theme().info(),
        ToastVariant::Success => theme().success(),
        ToastVariant::Warning => theme().warning(),
        ToastVariant::Error => theme().error(),
    }
}

/// The on-screen rect of each visible toast, oldest first. The draw and the
/// mouse hit-test both derive from this one geometry, so a click or hover can
/// never miss what's painted. Stacking stops once a toast no longer fits the
/// remaining height.
pub(crate) fn toast_rects(app: &App, area: Rect) -> Vec<Rect> {
    let width = card_width(area.width);
    if width <= 4 {
        return Vec::new();
    }
    let x = area.right().saturating_sub(TOAST_RIGHT_INSET + width);
    let mut y = area.y + 1;
    let mut rects = Vec::new();
    for toast in app.toasts.items() {
        let lines = crate::tui::entry_rows::wrap_text(
            &toast.message,
            (width - 4) as usize,
            TOAST_MAX_LINES,
        );
        // Flat chrome shows the countdown on its blank bottom padding row;
        // bordered chrome's bottom row is the border, so it grows one row to
        // give the countdown a dedicated line above the bottom border.
        let countdown_row = u16::from(!flat_chrome());
        let height = clamp_u16(lines.len()) + 2 + countdown_row;
        if y + height > area.bottom() {
            break;
        }
        rects.push(Rect::new(x, y, width, height));
        y += height + 1;
    }
    rects
}

/// The index of the toast under `(col, row)`, if any.
pub(crate) fn toast_at_point(app: &App, area: Rect, col: u16, row: u16) -> Option<usize> {
    toast_rects(app, area)
        .into_iter()
        .position(|rect| point_in_rect(rect, col, row))
}

/// Draw the toast stack in the top-right corner, oldest at the top with a blank
/// row between toasts. Runs at the very end of the frame — after overlays and
/// the scrim — so notifications stay readable over everything.
pub(crate) fn draw_toasts(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    for (index, (toast, rect)) in app
        .toasts
        .items()
        .iter()
        .zip(toast_rects(app, area))
        .enumerate()
    {
        let lines = crate::tui::entry_rows::wrap_text(
            &toast.message,
            rect.width.saturating_sub(4) as usize,
            TOAST_MAX_LINES,
        );
        let hovered = app.hover == crate::tui::state::HoverTarget::Toast(index);
        draw_toast(
            frame,
            rect,
            toast.variant,
            &lines,
            hovered,
            toast.remaining_fraction(),
        );
    }
}

/// One toast box. Flat chrome paints a panel-colored card with thick `┃` edge
/// columns in the variant's hue; bordered chrome draws a plain box with the
/// variant-colored border. Both keep one padding column inside the edges and
/// one padding row above and below the text. A hovered toast lifts to the
/// hover surface as the click-to-dismiss affordance. `progress` (1.0 → 0.0)
/// drives the countdown line on the bottom edge.
fn draw_toast(
    frame: &mut Frame<'_>,
    area: Rect,
    variant: ToastVariant,
    lines: &[String],
    hovered: bool,
    progress: f32,
) {
    let accent = toast_style(variant);
    let text: Vec<Line<'static>> = lines
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), theme().text())))
        .collect();
    let content = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(2),
    };
    if flat_chrome() {
        // The element surface, not the content one: toasts float over panels
        // that already carry `content_bg`, so on the same color only the edge
        // stripes would separate them.
        clear_surface(frame, area, theme().raised_bg());
        if hovered {
            frame.buffer_mut().set_style(area, theme().hover());
        }
        for edge_x in [area.x, area.right().saturating_sub(1)] {
            let edge = theme().glyphs().toast_edge.to_string();
            let stripe: Vec<Line<'static>> = (0..area.height)
                .map(|_| Line::from(Span::styled(edge.clone(), accent)))
                .collect();
            frame.render_widget(
                Paragraph::new(stripe),
                Rect {
                    x: edge_x,
                    width: 1,
                    ..area
                },
            );
        }
    } else {
        clear_surface(frame, area, theme().content_bg());
        if hovered {
            frame.buffer_mut().set_style(area, theme().hover());
        }
        frame.render_widget(
            Block::default().borders(Borders::ALL).border_style(accent),
            area,
        );
    }
    frame.render_widget(Paragraph::new(text), content);
    draw_countdown(frame, area, accent, progress);
}

/// The dismissal countdown: a thin line whose filled span shrinks left→right as
/// `progress` (time remaining, 1.0 → 0.0) drains. The elapsed span is left
/// blank so the line visibly gets shorter. It's inset one column inside the
/// edges on each side, aligning with the message text. Flat chrome draws it on
/// the blank bottom padding row; bordered chrome on the dedicated row just above
/// its bottom border.
fn draw_countdown(frame: &mut Frame<'_>, area: Rect, accent: Style, progress: f32) {
    let inner = area.width.saturating_sub(4);
    if inner == 0 {
        return;
    }
    // `ceil` keeps a freshly-pushed toast full-width (progress ≈ 1.0) rather
    // than dropping a column to rounding.
    let filled = (f32::from(inner) * progress).ceil() as u16;
    if filled == 0 {
        return;
    }
    let bottom_inset = if flat_chrome() { 1 } else { 2 };
    let glyph = theme().glyphs().toast_progress.to_string();
    let line = glyph.repeat(filled.min(inner) as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(line, accent))),
        Rect {
            x: area.x + 2,
            y: area.bottom().saturating_sub(bottom_inset),
            width: filled.min(inner),
            height: 1,
        },
    );
}
