use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
};

/// Width of the "Enter Password" container box, clamped to the available width.
const CONTAINER_WIDTH: u16 = 68;
/// Height of the container: border + padding + the sub-field box + a gap + the
/// error row + padding. The error row is reserved even when empty so the box
/// keeps a stable size across a wrong-passphrase retry.
const CONTAINER_HEIGHT: u16 = 9;
/// Height of the inner sub-field: faint border + one input row + faint border.
const SUBFIELD_HEIGHT: u16 = 3;

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

    // Center the container box within the outer border.
    let [group] = Layout::vertical([Constraint::Length(CONTAINER_HEIGHT.min(inner.height))])
        .flex(Flex::Center)
        .areas(inner);
    let [container_box] =
        Layout::horizontal([Constraint::Length(CONTAINER_WIDTH.min(inner.width))])
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
        Constraint::Length(1),
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
    // hint normally, replaced by the error message after a wrong passphrase.
    let status = error.unwrap_or("Enter your passphrase to unlock");
    frame.render_widget(
        Paragraph::new(Span::styled(
            status.to_string(),
            Style::default().add_modifier(Modifier::DIM),
        ))
        .alignment(Alignment::Center),
        error_row,
    );
}

/// The masked passphrase, echoing one `*` per typed character (never the raw
/// passphrase) with a trailing block caret — the same reversed-cell idiom the
/// search field uses (`search_field_title`).
fn masked_field(input: &str, caret_visible: bool) -> Line<'static> {
    let caret_style = if caret_visible {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::raw("*".repeat(input.chars().count())),
        Span::styled(" ", caret_style),
    ])
}
