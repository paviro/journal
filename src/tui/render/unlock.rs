use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

use super::caret_style;
use crate::tui::entry_rows::wrap_text;

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
/// carries the same blinking block caret as the search field. A standing hint
/// sits below the field, replaced by the error message after a wrong passphrase.
pub(crate) fn draw_unlock(
    frame: &mut Frame<'_>,
    input: &str,
    error: Option<&str>,
    caret_visible: bool,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(" Unlock Journal "))
        .title_bottom(Line::from(" enter unlock · esc quit ").right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
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

    // The container: bordered, titled top-left, generous padding around its
    // contents.
    let container = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(" Enter Password "))
        .padding(Padding::new(2, 2, 1, 1));
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
    // masked passphrase, with a padded input row inside.
    let subfield = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().add_modifier(Modifier::DIM))
        .padding(Padding::horizontal(1));
    let subfield_inner = subfield.inner(subfield_box);
    frame.render_widget(subfield, subfield_box);
    frame.render_widget(
        Paragraph::new(masked_field(input, caret_visible)),
        subfield_inner,
    );

    // Status line below the field, centered within the container: a standing
    // hint normally, replaced by the error message after a wrong passphrase. It
    // wraps so a narrow terminal shows the whole message across the rows the
    // container reserved for it.
    frame.render_widget(
        Paragraph::new(Span::styled(
            status.to_string(),
            Style::default().add_modifier(Modifier::DIM),
        ))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true }),
        error_row,
    );
}

/// The masked passphrase, echoing one `*` per typed character (never the raw
/// passphrase) with a trailing block caret — the same reversed-cell idiom the
/// search field uses (`search_field_title`).
fn masked_field(input: &str, caret_visible: bool) -> Line<'static> {
    Line::from(vec![
        Span::raw("*".repeat(input.chars().count())),
        Span::styled(" ", caret_style(caret_visible)),
    ])
}
