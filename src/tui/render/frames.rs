//! Dialog and full-screen modal frames, plus the shared yes/no confirm
//! buttons: the chrome every overlay draws before its own content.

use ratatui::{
    Frame,
    layout::{Alignment, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::surface::{point_in_rect, surface_content_inner};
use crate::tui::theme::theme;

use super::chrome::{centered_rect_fixed_size, clear_surface, flat_chrome};
use super::footer::key_chip_style;

/// Rows a dialog's frame consumes above and below its content: the two border
/// rows when bordered; flat trades them for a padding row, the title row, and
/// a blank row below the title on top, plus a padding row below the content —
/// so nothing sits on the card's edge and the title breathes. Sizing helpers
/// add this to their content rows.
pub(crate) fn dialog_frame_rows() -> u16 {
    if flat_chrome() { 4 } else { 2 }
}

/// A dialog's content rect within its outer `area`. Draw functions and mouse
/// hit-tests both derive geometry from this one place, so they can never
/// drift apart. Bordered chrome insets by the border; flat chrome trades the
/// side borders for a wider breathing margin, with a blank padding row above
/// the title and below the content.
pub(crate) fn dialog_inner(area: Rect) -> Rect {
    // Saturating per-axis (unlike `Rect::inner`, which zeroes the whole rect):
    // sizing helpers probe with height-1 rects and still need the real width.
    let top = if flat_chrome() { 3 } else { 1 };
    let frame_inner = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(top),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(dialog_frame_rows()),
    };
    surface_content_inner(frame_inner)
}

/// Clear and frame a dialog, returning its content rect (always
/// [`dialog_inner`] of `area`). Bordered chrome draws the classic titled box;
/// flat chrome paints a dialog-colored surface with a bold title row and, when
/// `esc_hint` is set, a muted `esc` dismiss hint on the right.
pub(crate) fn draw_dialog_frame(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    esc_hint: bool,
) -> Rect {
    clear_surface(frame, area, theme().dialog_bg());
    let title = title.trim();
    if flat_chrome() {
        // The title sits below a blank padding row, off the card's edge.
        let content = dialog_inner(area);
        let top = Rect {
            x: content.x,
            y: area.y + 1.min(area.height.saturating_sub(1)),
            width: content.width,
            height: 1.min(area.height),
        };
        if !title.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(title.to_string(), theme().heading())),
                top,
            );
        }
        if esc_hint {
            frame.render_widget(
                Paragraph::new(Span::styled("esc", theme().muted())).alignment(Alignment::Right),
                top,
            );
        }
    } else {
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_set(theme().glyphs().borders.border_set())
            .border_style(theme().dialog_border());
        if !title.is_empty() {
            block = block.title(format!(" {title} "));
        }
        frame.render_widget(block, area);
    }
    dialog_inner(area)
}

/// Width and gap of the two confirm buttons; sized for a comfortable click target
/// with room for the label and its key hint.
const CONFIRM_BUTTON_WIDTH: u16 = 16;

const CONFIRM_BUTTON_GAP: u16 = 2;

/// The `(yes, no)` button rects, centered on the last row of `inner`. Sizing and
/// hit-testing both derive from this, so the drawn buttons match the click targets.
pub(crate) fn confirm_button_rects(inner: Rect) -> (Rect, Rect) {
    let y = inner.y + inner.height.saturating_sub(1);
    let total = CONFIRM_BUTTON_WIDTH * 2 + CONFIRM_BUTTON_GAP;
    let start = inner.x + inner.width.saturating_sub(total) / 2;
    let yes = Rect {
        x: start,
        y,
        width: CONFIRM_BUTTON_WIDTH,
        height: 1,
    };
    let no = Rect {
        x: start + CONFIRM_BUTTON_WIDTH + CONFIRM_BUTTON_GAP,
        ..yes
    };
    (yes, no)
}

/// Draw the two confirm buttons as reversed + bold chips on the last row of
/// `inner`. The hovered button underlines as the click affordance — the chips
/// are already filled/reversed, so a surface change wouldn't read.
pub(crate) fn render_confirm_buttons(
    frame: &mut Frame<'_>,
    inner: Rect,
    yes_label: &str,
    no_label: &str,
    hovered: Option<bool>,
) {
    let (yes, no) = confirm_button_rects(inner);
    for (area, label, is_yes) in [(yes, yes_label, true), (no, no_label, false)] {
        // Flat chrome draws opencode-style filled chips; bordered keeps the
        // bracketed reversed buttons. Same rects either way, so the click
        // targets from `confirm_button_rects` stay valid.
        let (text, mut style) = if flat_chrome() {
            (format!(" {label} "), theme().button())
        } else {
            (format!("[ {label} ]"), key_chip_style())
        };
        if hovered == Some(is_yes) {
            style = style.patch(theme().button_hover());
        }
        frame.render_widget(
            Paragraph::new(Span::styled(text, style)).alignment(Alignment::Center),
            area,
        );
    }
}

/// Map a click to a confirm button: `Some(true)` for yes, `Some(false)` for no.
pub(crate) fn confirm_button_at(inner: Rect, col: u16, row: u16) -> Option<bool> {
    let (yes, no) = confirm_button_rects(inner);
    if point_in_rect(yes, col, row) {
        Some(true)
    } else if point_in_rect(no, col, row) {
        Some(false)
    } else {
        None
    }
}

/// Draw the internal editor's "Discard changes?" confirmation as a centered
/// modal, matching the confirm-delete dialog's look.
pub(crate) fn draw_editor_discard_confirm(frame: &mut Frame<'_>, hovered_button: Option<bool>) {
    let area = editor_discard_confirm_area(frame.area());
    let inner = draw_dialog_frame(frame, area, "Discard Changes", true);
    let line = Rect {
        y: inner.y,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new("Discard unsaved changes?").alignment(Alignment::Center),
        line,
    );
    render_confirm_buttons(frame, inner, "Discard (y)", "Keep (n)", hovered_button);
}

pub(crate) fn editor_discard_confirm_area(frame_area: Rect) -> Rect {
    // Message + blank + buttons, inside the frame.
    centered_rect_fixed_size(42, 3 + dialog_frame_rows(), frame_area)
}

pub(crate) fn editor_discard_choice_at_point(frame_area: Rect, col: u16, row: u16) -> Option<bool> {
    let inner = dialog_inner(editor_discard_confirm_area(frame_area));
    confirm_button_at(inner, col, row)
}

/// Draw the full-screen "journal chrome" frame shared by the startup modals
/// (unlock, device-access request, and the enroll/awaiting/disable notices)
/// and the image viewer: a bordered block titled top-left with the screen
/// name and, when non-empty, `status` bottom-left and `key_hint` bottom-right.
/// Clears the screen first and returns the inner area to lay the modal's
/// content into.
pub(crate) fn draw_modal_frame(
    frame: &mut Frame<'_>,
    title: &str,
    status: &str,
    key_hint: &str,
) -> Rect {
    let area = frame.area();
    clear_surface(frame, area, theme().base_bg());

    if flat_chrome() {
        // No outer border: the screen name and hints sit on full-width
        // element-surface bars along the top and bottom, like status bars.
        let bar = Style::default().bg(theme().element_bg());
        let top_bar = Rect {
            height: 1.min(area.height),
            ..area
        };
        frame.buffer_mut().set_style(top_bar, bar);
        let top = Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(2),
            ..top_bar
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {title} "), theme().muted())),
            top,
        );
        if area.height > 1 && (!status.is_empty() || !key_hint.is_empty()) {
            let bottom_bar = Rect {
                y: area.y + area.height - 1,
                ..top_bar
            };
            frame.buffer_mut().set_style(bottom_bar, bar);
            let bottom = Rect {
                y: bottom_bar.y,
                ..top
            };
            if !status.is_empty() {
                frame.render_widget(
                    Paragraph::new(Span::styled(format!(" {status} "), theme().muted())),
                    bottom,
                );
            }
            if !key_hint.is_empty() {
                frame.render_widget(
                    Paragraph::new(Span::styled(format!(" {key_hint} "), theme().muted()))
                        .alignment(Alignment::Right),
                    bottom,
                );
            }
        }
        return area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
    }

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_set(theme().glyphs().borders.border_set())
        .border_style(theme().dialog_border())
        .title_top(Line::from(format!(" {title} ")));
    if !status.is_empty() {
        block = block.title_bottom(Line::from(format!(" {status} ")));
    }
    if !key_hint.is_empty() {
        block = block.title_bottom(Line::from(format!(" {key_hint} ")).right_aligned());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}
