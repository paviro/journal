use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::{App, Focus, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HintId {
    NewJournal,
    NewEntry,
    BeginSearch,
    Quit,
    EditSelected,
    ViewSelected,
    BeginDelete,
    BeginEditTags,
    BeginEditPeople,
    BeginEditActivities,
    BeginEditFeelings,
    BeginEditMood,
    ExitSearch,
    CancelOverlay,
    TagsToggle,
    TagsSwitchFocus,
    TagsAddFromInput,
    TagsSave,
    FeelingsToggle,
    FeelingsSave,
    MoodDecrease,
    MoodIncrease,
    MoodSave,
    MoodClear,
    OpenImageViewer,
    HintsToggle,
    ToggleJournals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hint {
    pub(crate) label: &'static str,
    pub(crate) key_hint: &'static str,
    pub(crate) id: HintId,
}

impl Hint {
    pub(crate) const fn new(label: &'static str, key_hint: &'static str, id: HintId) -> Self {
        Self {
            label,
            key_hint,
            id,
        }
    }

    fn text(self) -> String {
        format!("{} ({})", self.label, self.key_hint)
    }
}

#[derive(Debug, Clone)]
struct RenderedHintLine {
    text: String,
    hint_origin: u16,
    hints: Vec<Hint>,
}

pub(crate) fn hints_text(hints: &[Hint]) -> String {
    hints
        .iter()
        .copied()
        .map(Hint::text)
        .collect::<Vec<_>>()
        .join(" | ")
}

pub(crate) fn hint_id_at(hints: &[Hint], origin_x: u16, col: u16) -> Option<HintId> {
    if col < origin_x {
        return None;
    }
    let rel = col.saturating_sub(origin_x) as usize;
    let mut x = 0usize;
    for hint in hints.iter().copied() {
        let text = hint.text();
        let width = UnicodeWidthStr::width(text.as_str());
        if rel >= x && rel < x + width {
            return Some(hint.id);
        }
        x += width + 3;
    }
    None
}

pub(crate) fn hint_lines(hints: &[Hint], width: u16) -> Vec<Line<'static>> {
    rendered_hint_lines(hints, width)
        .into_iter()
        .map(|line| Line::from(line.text))
        .collect()
}

pub(crate) fn hint_height(hints: &[Hint], width: u16) -> u16 {
    rendered_hint_lines(hints, width)
        .len()
        .max(1)
        .min(u16::MAX as usize) as u16
}

pub(crate) fn hint_id_at_wrapped(
    hints: &[Hint],
    origin_x: u16,
    origin_y: u16,
    width: u16,
    col: u16,
    row: u16,
) -> Option<HintId> {
    let relative_row = row.checked_sub(origin_y)? as usize;
    let lines = rendered_hint_lines(hints, width);
    let line = lines.get(relative_row)?;
    hint_id_at(&line.hints, origin_x.saturating_add(line.hint_origin), col)
}

fn rendered_hint_lines(hints: &[Hint], width: u16) -> Vec<RenderedHintLine> {
    wrapped_hint_rows(hints, width)
        .into_iter()
        .map(|hints| RenderedHintLine {
            text: hints_text(&hints),
            hint_origin: 0,
            hints,
        })
        .collect()
}

fn wrapped_hint_rows(hints: &[Hint], width: u16) -> Vec<Vec<Hint>> {
    let available = width as usize;
    let mut rows: Vec<Vec<Hint>> = Vec::new();
    let mut row: Vec<Hint> = Vec::new();
    let mut row_width = 0usize;

    for hint in hints.iter().copied() {
        let hint_width = UnicodeWidthStr::width(hint.text().as_str());
        let separator_width = if row.is_empty() { 0 } else { 3 };
        if !row.is_empty() && row_width + separator_width + hint_width > available {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        if !row.is_empty() {
            row_width += 3;
        }
        row_width += hint_width;
        row.push(hint);
    }

    if !row.is_empty() {
        rows.push(row);
    }

    rows
}

#[cfg(test)]
pub(crate) fn footer_text(app: &App) -> String {
    if !app.status().is_empty() {
        return app.status().to_string();
    }

    match app.nav.mode {
        Mode::Search => search_footer_line(app).text(),
        Mode::Browse => browse_footer_line(app).text(),
    }
}

pub(crate) fn footer_lines(app: &App, width: u16) -> Text<'static> {
    if !app.status().is_empty() {
        return Text::from(app.status().to_string());
    }
    if !app.config.show_hints {
        return Text::default();
    }

    let lines = match app.nav.mode {
        Mode::Search => search_footer_line(app).lines(width),
        Mode::Browse => browse_footer_line(app).lines(width),
    };
    Text::from(lines)
}

pub(crate) fn footer_height(app: &App, width: u16) -> u16 {
    if !app.status().is_empty() {
        return 1;
    }
    if !app.config.show_hints {
        return 0;
    }

    match app.nav.mode {
        Mode::Search => search_footer_line(app).height(width),
        Mode::Browse => browse_footer_line(app).height(width),
    }
}

#[cfg(test)]
pub(crate) fn footer_hint_id_at(app: &App, origin_x: u16, col: u16) -> Option<HintId> {
    if !app.status().is_empty() {
        return None;
    }

    match app.nav.mode {
        Mode::Search => search_footer_line(app).hint_id_at(origin_x, col),
        Mode::Browse => browse_footer_line(app).hint_id_at(origin_x, col),
    }
}

pub(crate) fn footer_hint_id_at_point(
    app: &App,
    origin_x: u16,
    origin_y: u16,
    width: u16,
    col: u16,
    row: u16,
) -> Option<HintId> {
    if !app.status().is_empty() || !app.config.show_hints {
        return None;
    }

    match app.nav.mode {
        Mode::Search => {
            search_footer_line(app).hint_id_at_point(origin_x, origin_y, width, col, row)
        }
        Mode::Browse => {
            browse_footer_line(app).hint_id_at_point(origin_x, origin_y, width, col, row)
        }
    }
}

#[cfg(test)]
pub(crate) fn expanded_footer_text(app: &App) -> String {
    hints_text(&expanded_footer_hints(app))
}

pub(crate) fn expanded_footer_lines(app: &App, width: u16) -> Text<'static> {
    if !app.config.show_hints {
        return Text::default();
    }
    Text::from(hint_lines(
        &expanded_footer_hints(app),
        width.saturating_sub(1),
    ))
}

pub(crate) fn expanded_footer_height(app: &App, width: u16) -> u16 {
    if !app.config.show_hints {
        return 0;
    }
    hint_height(&expanded_footer_hints(app), width.saturating_sub(1))
}

pub(crate) fn expanded_footer_hint_id_at_point(
    app: &App,
    origin_x: u16,
    origin_y: u16,
    width: u16,
    col: u16,
    row: u16,
) -> Option<HintId> {
    if !app.config.show_hints {
        return None;
    }
    hint_id_at_wrapped(
        &expanded_footer_hints(app),
        origin_x.saturating_add(1),
        origin_y,
        width.saturating_sub(1),
        col,
        row,
    )
}

#[derive(Debug, Clone)]
struct HintLine {
    prefix: Option<String>,
    hints: Vec<Hint>,
}

impl HintLine {
    fn rendered_lines(&self, width: u16) -> Vec<RenderedHintLine> {
        if self.hints.is_empty() {
            return self
                .prefix
                .as_ref()
                .map(|prefix| {
                    vec![RenderedHintLine {
                        text: prefix.clone(),
                        hint_origin: 0,
                        hints: Vec::new(),
                    }]
                })
                .unwrap_or_default();
        }

        let Some(prefix) = &self.prefix else {
            return rendered_hint_lines(&self.hints, width);
        };

        let prefix_width = UnicodeWidthStr::width(prefix.as_str()).min(u16::MAX as usize) as u16;
        let first_hint_width = self
            .hints
            .first()
            .map(|hint| UnicodeWidthStr::width(hint.text().as_str()).min(u16::MAX as usize) as u16)
            .unwrap_or(0);

        let mut lines = Vec::new();
        let mut remaining_hints = self.hints.as_slice();
        if prefix_width
            .saturating_add(3)
            .saturating_add(first_hint_width)
            <= width
        {
            let first_width = width.saturating_sub(prefix_width).saturating_sub(3);
            let first_rows = wrapped_hint_rows(remaining_hints, first_width);
            if let Some(first_row) = first_rows.first() {
                let consumed = first_row.len();
                lines.push(RenderedHintLine {
                    text: format!("{prefix} | {}", hints_text(first_row)),
                    hint_origin: prefix_width.saturating_add(3),
                    hints: first_row.clone(),
                });
                remaining_hints = &remaining_hints[consumed..];
            }
        } else {
            lines.push(RenderedHintLine {
                text: prefix.clone(),
                hint_origin: 0,
                hints: Vec::new(),
            });
        }

        lines.extend(rendered_hint_lines(remaining_hints, width));
        lines
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        self.rendered_lines(width)
            .into_iter()
            .map(|line| Line::from(line.text))
            .collect()
    }

    fn height(&self, width: u16) -> u16 {
        self.rendered_lines(width)
            .len()
            .max(1)
            .min(u16::MAX as usize) as u16
    }

    #[cfg(test)]
    fn text(&self) -> String {
        let hints = hints_text(&self.hints);
        match (&self.prefix, hints.is_empty()) {
            (Some(prefix), false) => format!("{prefix} | {hints}"),
            (Some(prefix), true) => prefix.clone(),
            (None, false) => hints,
            (None, true) => String::new(),
        }
    }

    #[cfg(test)]
    fn hint_id_at(&self, origin_x: u16, col: u16) -> Option<HintId> {
        let hint_origin = origin_x.saturating_add(
            self.prefix
                .as_ref()
                .map(|prefix| {
                    UnicodeWidthStr::width(prefix.as_str()).min(u16::MAX as usize) as u16 + 3
                })
                .unwrap_or(0),
        );
        hint_id_at(&self.hints, hint_origin, col)
    }

    fn hint_id_at_point(
        &self,
        origin_x: u16,
        origin_y: u16,
        width: u16,
        col: u16,
        row: u16,
    ) -> Option<HintId> {
        let relative_row = row.checked_sub(origin_y)? as usize;
        let lines = self.rendered_lines(width);
        let line = lines.get(relative_row)?;
        hint_id_at(&line.hints, origin_x.saturating_add(line.hint_origin), col)
    }
}

fn search_footer_line(app: &App) -> HintLine {
    // The query now lives on the entry panel's top-right border (see
    // `draw_entry_list`), so the footer only carries the action hints.
    let hints = match app.nav.focus {
        Focus::EntryView if app.has_selected_entry_target() => {
            let mut hints = selected_entry_action_hints(true);
            hints.extend(image_hint(app));
            hints.push(Hint::new("exit search", "esc", HintId::ExitSearch));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::EntryView => vec![
            Hint::new("exit search", "esc", HintId::ExitSearch),
            Hint::new("quit", "q", HintId::Quit),
        ],
        _ => {
            let mut hints = Vec::new();
            if app.has_selected_entry_target() {
                hints.push(Hint::new("view", "enter", HintId::ViewSelected));
            }
            hints.push(Hint::new("exit search", "esc", HintId::ExitSearch));
            hints
        }
    };

    HintLine {
        prefix: None,
        hints,
    }
}

fn browse_footer_line(app: &App) -> HintLine {
    let hints = match app.nav.focus {
        Focus::Journals => vec![
            Hint::new("new journal", "n", HintId::NewJournal),
            Hint::new("search", "/", HintId::BeginSearch),
            journals_hint(app),
            Hint::new("hints", "h", HintId::HintsToggle),
            Hint::new("quit", "q", HintId::Quit),
        ],
        Focus::Entries => {
            let mut hints = vec![Hint::new("new entry", "n", HintId::NewEntry)];
            if app.has_selected_entry_target() {
                hints.extend(selected_entry_action_hints(true));
            }
            // The image viewer opens only from a focused entry view, so no
            // `images` hint here.
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::EntryView => {
            let mut hints = vec![Hint::new("new entry", "n", HintId::NewEntry)];
            if app.has_selected_entry_target() {
                hints.extend(selected_entry_action_hints(true));
            }
            hints.extend(image_hint(app));
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(journals_hint(app));
            hints.push(Hint::new("hints", "h", HintId::HintsToggle));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
    };

    HintLine {
        prefix: None,
        hints,
    }
}

/// The `images (i)` hint, shown only when the selected entry has images.
fn image_hint(app: &App) -> Option<Hint> {
    (app.selected_entry_image_count() > 0).then_some(Hint::new(
        "images",
        "i",
        HintId::OpenImageViewer,
    ))
}

fn journals_hint(app: &App) -> Hint {
    let label = if app.config.show_journals {
        "hide journals"
    } else {
        "journals"
    };
    Hint::new(label, "j", HintId::ToggleJournals)
}

fn selected_entry_action_hints(include_view: bool) -> Vec<Hint> {
    let mut hints = Vec::new();
    hints.push(Hint::new("edit", "e", HintId::EditSelected));
    if include_view {
        hints.push(Hint::new("view", "enter", HintId::ViewSelected));
    }
    hints.push(Hint::new("del", "d", HintId::BeginDelete));
    hints.push(Hint::new("tags", "t", HintId::BeginEditTags));
    hints.push(Hint::new("feel", "f", HintId::BeginEditFeelings));
    hints.push(Hint::new("mood", "m", HintId::BeginEditMood));
    hints
}

fn expanded_footer_hints(app: &App) -> Vec<Hint> {
    let mut hints = Vec::new();
    if app.nav.mode == Mode::Browse {
        hints.push(Hint::new("new entry", "n", HintId::NewEntry));
    }
    if app.has_selected_entry_target() {
        hints.push(Hint::new("edit", "e", HintId::EditSelected));
        hints.push(Hint::new("close", "enter/esc", HintId::CancelOverlay));
        hints.push(Hint::new("del", "d", HintId::BeginDelete));
        hints.push(Hint::new("tags", "t", HintId::BeginEditTags));
        hints.push(Hint::new("ppl", "p", HintId::BeginEditPeople));
        hints.push(Hint::new("act", "a", HintId::BeginEditActivities));
        hints.push(Hint::new("feel", "f", HintId::BeginEditFeelings));
        hints.push(Hint::new("mood", "m", HintId::BeginEditMood));
        hints.extend(image_hint(app));
    } else {
        hints.push(Hint::new("close", "enter/esc", HintId::CancelOverlay));
    }
    if app.nav.mode == Mode::Browse {
        hints.push(Hint::new("search", "/", HintId::BeginSearch));
    }
    hints.push(Hint::new("hints", "h", HintId::HintsToggle));
    hints.push(Hint::new("quit", "q", HintId::Quit));
    hints
}

pub(crate) fn panel_block(
    title: &str,
    focused: bool,
    footer_label: Option<String>,
) -> Block<'static> {
    let mut block = Block::default()
        .title(panel_title(title, focused))
        .borders(Borders::ALL);

    if focused {
        block = block
            .border_type(BorderType::Thick)
            .border_style(Style::default().add_modifier(Modifier::BOLD));
    }

    if let Some(label) = footer_label {
        block = block.title_bottom(Line::from(format!(" {label} ")).right_aligned());
    }

    block
}

/// Draw a dimmed message centered both horizontally and vertically within a
/// panel's content area — used for empty states like "No entry selected" and
/// "No results".
pub(crate) fn render_centered_notice(frame: &mut Frame<'_>, content: Rect, message: &str) {
    if content.width == 0 || content.height == 0 {
        return;
    }
    let line = Rect {
        y: content.y + content.height.saturating_sub(1) / 2,
        height: 1,
        ..content
    };
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::DIM)),
        line,
    );
}

pub(crate) fn count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

pub(crate) fn panel_title(title: &str, focused: bool) -> Line<'static> {
    let label = format!(" {title} ");
    if focused {
        Line::from(Span::styled(
            label,
            Style::default().add_modifier(Modifier::REVERSED),
        ))
    } else {
        Line::from(label)
    }
}

pub(crate) fn render_vertical_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut ScrollbarState,
) {
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        state,
    );
}

pub(crate) fn render_scrollbar_if_needed(
    frame: &mut Frame<'_>,
    area: Rect,
    total_height: usize,
    viewport_height: u16,
    scroll: usize,
) {
    if total_height > viewport_height as usize {
        let mut state = ScrollbarState::default()
            .content_length(total_height)
            .viewport_content_length(viewport_height as usize)
            .position(crate::tui::scroll::scrollbar_position(
                scroll,
                total_height,
                viewport_height,
            ));
        render_vertical_scrollbar(frame, area, &mut state);
    }
}

pub(crate) fn centered_rect_fixed_size(width: u16, height: u16, area: Rect) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(row);
    cell
}
