use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Padding, Paragraph, Wrap},
};

use journal_storage::PendingRequest;

use crate::tui::entry_rows::wrap_text;

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
    let inner = super::draw_modal_frame(frame, "Device access request", "y approve · n deny · esc later");
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let recipient = &request.recipient;

    let dim = Style::default().add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let info_lines = vec![
        Line::from(vec![
            Span::raw("Device "),
            Span::styled(format!("'{}'", recipient.name), bold),
            Span::raw(" requests access."),
        ]),
        Line::from(vec![
            Span::styled("fingerprint: ", dim),
            Span::styled(recipient.fingerprint(), bold),
        ]),
        Line::from(Span::styled(
            "Confirm this matches what the joining device shows before approving.",
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

/// Which unreadable-store situation [`draw_pending_notice`] explains.
pub(crate) enum AccessNotice {
    /// A join request is queued — waiting for another device to approve it.
    AwaitingApproval,
    /// No usable key, so the user must enroll. `retired_key` is true when this
    /// launch just renamed a now-dead (revoked) key aside, false for a device
    /// that was never enrolled and so never had one.
    NeedsEnroll { retired_key: bool },
}

/// Draw the full-screen notice a device sees when it can't decrypt this encrypted
/// store — either awaiting approval of its join request, or holding no usable key
/// and needing to enroll. See [`AccessNotice`].
pub(crate) fn draw_pending_notice(frame: &mut Frame<'_>, device_name: &str, notice: &AccessNotice) {
    let area = super::draw_modal_frame(frame, "Journal", "any key to exit");
    if area.height == 0 || area.width == 0 {
        return;
    }

    // Reads naturally whether or not this device has a name yet: a keyless device
    // (never enrolled) has none, so it becomes the sentence subject "This device".
    let subject = if device_name.is_empty() {
        "This device".to_string()
    } else {
        format!("Device '{device_name}'")
    };
    let dim = Style::default().add_modifier(Modifier::DIM);

    let container_width = CONTAINER_WIDTH.min(area.width);
    // The width the prose wraps to (borders + horizontal padding removed). Wrapping
    // here instead of hand-splitting sizes the box to the real line count.
    let text_width = container_width.saturating_sub(6) as usize;

    let (title, intro, instruction, command) = match notice {
        AccessNotice::AwaitingApproval => (
            " Awaiting approval ",
            format!("{subject} has requested access but isn't approved yet."),
            "Approve it from a device that can already read this journal:".to_string(),
            format!("journal encryption device approve {device_name}"),
        ),
        AccessNotice::NeedsEnroll { .. } => (
            " Not authorized ",
            format!(
                "{subject} isn't a recipient of this journal, so it can't read any entries."
            ),
            "Run this to request access, then approve it from a device that can already read this journal:".to_string(),
            crate::ENROLL_CMD.to_string(),
        ),
    };

    let wrapped = |text: &str| {
        wrap_text(text, text_width, usize::MAX)
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>()
    };
    let mut lines = wrapped(&intro);
    // Only when a real key was just retired — a never-enrolled device had none.
    if matches!(notice, AccessNotice::NeedsEnroll { retired_key: true }) {
        lines.push(Line::from(""));
        lines.extend(wrapped(
            "This device's old key has been retired (renamed aside, recoverable).",
        ));
    }
    lines.push(Line::from(""));
    lines.extend(wrapped(&instruction));
    lines.push(Line::from(Span::styled(format!("  {command}"), dim)));

    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(title))
        .padding(Padding::new(2, 2, 1, 1));
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

/// Draw the full-screen notice shown when encryption was disabled on another
/// device: this device fell back to plaintext and retired its key and trust pins
/// (renamed aside, not deleted). Dismissed on any key.
pub(crate) fn draw_disable_notice(frame: &mut Frame<'_>) {
    let area = super::draw_modal_frame(frame, "Journal", "any key to continue");
    if area.height == 0 || area.width == 0 {
        return;
    }

    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);

    let container_width = CONTAINER_WIDTH.min(area.width);
    // Borders (1 each side) plus the 2-cell horizontal padding: the width the body
    // text wraps to. Wrapping it here (rather than hand-splitting) lets the box
    // size to the real line count, so the dismiss hint is never pushed off-screen.
    let text_width = container_width.saturating_sub(6) as usize;
    let body = "This journal is now plaintext. This device's encryption key and trust pins \
        have been retired — renamed aside next to the config, not deleted, so they can be \
        recovered if this was unexpected.";

    let mut lines = vec![
        Line::from(Span::styled(
            "Encryption was disabled on another device.",
            bold,
        )),
        Line::from(""),
    ];
    lines.extend(wrap_text(body, text_width, usize::MAX).into_iter().map(Line::from));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("To turn encryption back on, run "),
        Span::styled("journal encryption enable", dim),
        Span::raw("."),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(" Encryption disabled "))
        .padding(Padding::new(2, 2, 1, 1));
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
