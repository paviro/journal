use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::tui::entry_rows::wrap_text;
use crate::tui::text_input::PassphraseInput;
use crate::tui::theme::theme;

/// Width of the "Enter Password" container box, clamped to the available width.
const CONTAINER_WIDTH: u16 = 68;
/// Height of the inner sub-field: faint border + one input row + faint border.
const SUBFIELD_HEIGHT: u16 = 3;
/// Fixed rows of the container besides the status region: border (2) + vertical
/// padding (2) + the sub-field box + the gap above the status.
const CONTAINER_CHROME_HEIGHT: u16 = 2 + 2 + SUBFIELD_HEIGHT + 1;
/// Standing hint shown below the field until a wrong passphrase replaces it. The
/// status region is sized for whichever of this and the current message wraps to
/// more rows, so the box stays a stable height across a retry.
const HINT: &str = "Enter your passphrase to unlock";
/// Cap on wrapped status rows so a pathologically narrow terminal can't grow the
/// container without bound.
const MAX_STATUS_LINES: usize = 4;

/// Draw the fullscreen unlock screen shown at startup while an encrypted store
/// is still locked. The passphrase sits in a "Enter Password" container whose
/// input line has its own faint-bordered sub-field; it's masked with `*` and
/// carries the native bar cursor like every other text field. A standing hint
/// sits below the field, replaced by the error message after a wrong passphrase.
/// Returns the passphrase field's inner rect, so the unlock loop can map a
/// mouse click onto the caret. `None` when the screen is too small to draw it.
pub(crate) fn draw_unlock(
    frame: &mut Frame<'_>,
    input: &PassphraseInput,
    error: Option<&str>,
) -> Option<Rect> {
    let inner = super::draw_modal_frame(frame, "Unlock Journal", "enter unlock · esc quit");
    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    // Size the status region to whatever wraps to the most rows at this width, so
    // a narrow terminal grows the box downward instead of clipping the message.
    let container_width = CONTAINER_WIDTH.min(inner.width);
    let status = error.unwrap_or(HINT);
    let status_width = container_width.saturating_sub(6) as usize; // border + padding
    let status_lines = wrap_text(HINT, status_width, MAX_STATUS_LINES)
        .len()
        .max(wrap_text(status, status_width, MAX_STATUS_LINES).len())
        .max(1) as u16;
    let container_height = (CONTAINER_CHROME_HEIGHT + status_lines).min(inner.height);

    // Center the container box within the outer border.
    let [group] = Layout::vertical([Constraint::Length(container_height)])
        .flex(Flex::Center)
        .areas(inner);
    let [container_box] = Layout::horizontal([Constraint::Length(container_width)])
        .flex(Flex::Center)
        .areas(group);

    // The container: titled top-left, generous padding around its contents.
    let container = super::container_block("Enter Password");
    let container_inner = container.inner(container_box);
    frame.render_widget(container, container_box);

    // Stack the sub-field box and (below a gap) the error row inside the
    // container. The sub-field spans the container's inner width, so the
    // container's padding leaves a small margin on each side.
    let [subfield_box, _gap, error_row] = Layout::vertical([
        Constraint::Length(SUBFIELD_HEIGHT),
        Constraint::Length(1),
        Constraint::Length(status_lines),
    ])
    .areas(container_inner);

    // The input line's own sub-field: a faint (dimmed) border framing just the
    // masked passphrase, with a padded input row inside — or, in flat chrome,
    // an element-colored surface with the same inner geometry.
    let subfield = if super::flat_chrome() {
        Block::new()
            .style(Style::default().bg(theme().element_bg()))
            .padding(Padding::new(2, 2, 1, 1))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_set(theme().glyphs().borders.border_set())
            .border_style(theme().muted())
            .padding(Padding::horizontal(1))
    };
    let subfield_inner = subfield.inner(subfield_box);
    frame.render_widget(subfield, subfield_box);
    frame.render_widget(Paragraph::new(masked_field(input)), subfield_inner);
    // Native bar cursor at the caret, like every other text field.
    if subfield_inner.width > 0 {
        let col = (input.cursor() as u16).min(subfield_inner.width - 1);
        frame.set_cursor_position((subfield_inner.x + col, subfield_inner.y));
    }

    // Status line below the field, centered within the container: a standing
    // hint normally, replaced by the error message after a wrong passphrase. It
    // wraps so a narrow terminal shows the whole message across the rows the
    // container reserved for it.
    frame.render_widget(
        Paragraph::new(Span::styled(status.to_string(), theme().muted()))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        error_row,
    );

    Some(subfield_inner)
}

/// The masked passphrase: one `*` per typed character, never the raw text.
fn masked_field(input: &PassphraseInput) -> Line<'static> {
    Line::from("*".repeat(input.as_str().chars().count()))
}
