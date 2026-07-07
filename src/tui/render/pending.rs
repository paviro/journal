use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Padding, Paragraph, Wrap},
};

use journal_storage::PendingRequest;

/// Width of the request container, clamped to the available width.
const CONTAINER_WIDTH: u16 = 68;

/// Draw the device-access approval modal shown before the app loads when a
/// pending join request exists in the synced `.age/` folder. While approval runs,
/// `progress` carries `(done, total)` and the hint row becomes a re-encryption
/// gauge.
pub(crate) fn draw_pending_request(
    frame: &mut Frame<'_>,
    request: &PendingRequest,
    progress: Option<(usize, usize)>,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(" Device access request "))
        .title_bottom(Line::from(" y approve · n deny · esc later ").right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let recipient = &request.recipient;

    let dim = Style::default().add_modifier(Modifier::DIM);
    let info_lines = vec![
        Line::from(vec![
            Span::raw("Device "),
            Span::styled(
                format!("'{}'", recipient.name),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(" requests access."),
        ]),
        Line::from(Span::styled(
            format!("key: {}", key_preview(&recipient.key)),
            dim,
        )),
        Line::from(""),
        Line::from("Approve and re-encrypt all entries to this device?"),
    ];

    // Border (2) + vertical padding (2) + info lines + gap + bottom row.
    let container_width = CONTAINER_WIDTH.min(inner.width);
    let container_height = (info_lines.len() as u16 + 6).min(inner.height);
    let [group] = Layout::vertical([Constraint::Length(container_height)])
        .flex(Flex::Center)
        .areas(inner);
    let [container_box] = Layout::horizontal([Constraint::Length(container_width)])
        .flex(Flex::Center)
        .areas(group);

    let container = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(" Grant access "))
        .padding(Padding::new(2, 2, 1, 1));
    let container_inner = container.inner(container_box);
    frame.render_widget(container, container_box);

    let [info_area, _gap, bottom] = Layout::vertical([
        Constraint::Length(info_lines.len() as u16),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(container_inner);
    frame.render_widget(
        Paragraph::new(info_lines).wrap(Wrap { trim: true }),
        info_area,
    );

    match progress {
        Some((done, total)) => {
            let ratio = if total == 0 {
                1.0
            } else {
                (done as f64 / total as f64).clamp(0.0, 1.0)
            };
            frame.render_widget(
                Gauge::default()
                    .ratio(ratio)
                    .label(format!("Re-encrypting… {done}/{total}")),
                bottom,
            );
        }
        None => frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "y approve    n deny    esc later",
                dim,
            ))),
            bottom,
        ),
    }
}

/// Draw the full-screen notice a device sees when it has an identity but isn't a
/// store recipient, so it can't decrypt history. `awaiting` picks the wording: a
/// request still queued for approval, versus a device that isn't authorized and
/// has nothing pending (denied, removed, or never synced).
pub(crate) fn draw_pending_notice(frame: &mut Frame<'_>, device_name: &str, awaiting: bool) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let label = if device_name.is_empty() {
        "this device".to_string()
    } else {
        format!("'{device_name}'")
    };
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);

    let (title, mut lines) = if awaiting {
        (
            " Awaiting approval ",
            vec![
                Line::from(vec![
                    Span::raw("Device "),
                    Span::styled(label, bold),
                    Span::raw(" has requested access but isn't approved yet."),
                ]),
                Line::from(""),
                Line::from("Approve it from a device that can already read this journal:"),
                Line::from(Span::styled(
                    format!("  journal encryption device approve {device_name}"),
                    dim,
                )),
            ],
        )
    } else {
        (
            " Not authorized ",
            vec![
                Line::from(vec![
                    Span::raw("Device "),
                    Span::styled(label, bold),
                    Span::raw(" isn't a recipient of this journal, and no"),
                ]),
                Line::from("access request is queued — it may have been denied or removed,"),
                Line::from("or the request never synced."),
                Line::from(""),
                Line::from("To request access again, delete this device's identity and run:"),
                Line::from(Span::styled("  journal encryption device enroll", dim)),
            ],
        )
    };
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Press any key to exit.", dim)));

    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(title))
        .padding(Padding::new(2, 2, 1, 1));
    let container_width = CONTAINER_WIDTH.min(area.width);
    let container_height = (lines.len() as u16 + 4).min(area.height);
    let [group] = Layout::vertical([Constraint::Length(container_height)])
        .flex(Flex::Center)
        .areas(area);
    let [container_box] = Layout::horizontal([Constraint::Length(container_width)])
        .flex(Flex::Center)
        .areas(group);
    let inner = block.inner(container_box);
    frame.render_widget(block, container_box);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// A truncated preview of the public key, enough to eyeball against what the
/// other device printed without filling the modal with a full bech32 string.
fn key_preview(key: &str) -> String {
    let head: String = key.chars().take(24).collect();
    if head.len() < key.len() {
        format!("{head}…")
    } else {
        head
    }
}
