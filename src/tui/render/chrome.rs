use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Scrollbar, ScrollbarOrientation, ScrollbarState},
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

pub(crate) fn footer_text(app: &App) -> String {
    if !app.status().is_empty() {
        return app.status().to_string();
    }

    match app.mode {
        Mode::Search => search_footer_line(app).text(),
        Mode::Browse => browse_footer_line(app).text(),
    }
}

pub(crate) fn footer_hint_id_at(app: &App, origin_x: u16, col: u16) -> Option<HintId> {
    if !app.status().is_empty() {
        return None;
    }

    match app.mode {
        Mode::Search => search_footer_line(app).hint_id_at(origin_x, col),
        Mode::Browse => browse_footer_line(app).hint_id_at(origin_x, col),
    }
}

pub(crate) fn expanded_footer_text() -> String {
    format!(" {}", hints_text(&expanded_footer_hints()))
}

pub(crate) fn expanded_footer_hint_id_at(origin_x: u16, col: u16) -> Option<HintId> {
    hint_id_at(&expanded_footer_hints(), origin_x.saturating_add(1), col)
}

#[derive(Debug, Clone)]
struct HintLine {
    prefix: Option<String>,
    hints: Vec<Hint>,
}

impl HintLine {
    fn text(&self) -> String {
        let hints = hints_text(&self.hints);
        match (&self.prefix, hints.is_empty()) {
            (Some(prefix), false) => format!("{prefix} | {hints}"),
            (Some(prefix), true) => prefix.clone(),
            (None, false) => hints,
            (None, true) => String::new(),
        }
    }

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
}

fn search_footer_line(app: &App) -> HintLine {
    let query = format!("Search {}: {}", app.search_scope_label(), app.search.query);
    let hints = match app.focus {
        Focus::EntryView if app.has_selected_entry_target() => vec![
            Hint::new("view", "enter", HintId::ViewSelected),
            Hint::new("edit", "e", HintId::EditSelected),
            Hint::new("delete", "d", HintId::BeginDelete),
            Hint::new("tags", "t", HintId::BeginEditTags),
            Hint::new("feelings", "f", HintId::BeginEditFeelings),
            Hint::new("mood", "m", HintId::BeginEditMood),
            Hint::new("exit search", "esc", HintId::ExitSearch),
            Hint::new("quit", "q", HintId::Quit),
        ],
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
        prefix: Some(query),
        hints,
    }
}

fn browse_footer_line(app: &App) -> HintLine {
    let hints = match app.focus {
        Focus::Journals => vec![
            Hint::new("new journal", "n", HintId::NewJournal),
            Hint::new("search", "/", HintId::BeginSearch),
            Hint::new("quit", "q", HintId::Quit),
        ],
        Focus::Entries => {
            let mut hints = vec![Hint::new("new entry", "n", HintId::NewEntry)];
            if app.has_selected_entry_target() {
                hints.push(Hint::new("edit", "e", HintId::EditSelected));
                hints.push(Hint::new("view", "enter", HintId::ViewSelected));
                hints.push(Hint::new("delete", "d", HintId::BeginDelete));
                hints.push(Hint::new("tags", "t", HintId::BeginEditTags));
                hints.push(Hint::new("feelings", "f", HintId::BeginEditFeelings));
                hints.push(Hint::new("mood", "m", HintId::BeginEditMood));
            }
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
        Focus::EntryView => {
            let mut hints = vec![Hint::new("new entry", "n", HintId::NewEntry)];
            if app.has_selected_entry_target() {
                hints.push(Hint::new("edit", "e", HintId::EditSelected));
                hints.push(Hint::new("view", "enter", HintId::ViewSelected));
                hints.push(Hint::new("delete", "d", HintId::BeginDelete));
                hints.push(Hint::new("tags", "t", HintId::BeginEditTags));
                hints.push(Hint::new("feelings", "f", HintId::BeginEditFeelings));
                hints.push(Hint::new("mood", "m", HintId::BeginEditMood));
            }
            hints.push(Hint::new("search", "/", HintId::BeginSearch));
            hints.push(Hint::new("quit", "q", HintId::Quit));
            hints
        }
    };

    HintLine {
        prefix: None,
        hints,
    }
}

fn expanded_footer_hints() -> [Hint; 3] {
    [
        Hint::new("close", "enter/esc", HintId::CancelOverlay),
        Hint::new("edit", "e", HintId::EditSelected),
        Hint::new("quit", "q", HintId::Quit),
    ]
}

pub(crate) fn panel_block(title: &str, focused: bool, word_count: Option<usize>) -> Block<'static> {
    let mut block = Block::default()
        .title(panel_title(title, focused))
        .borders(Borders::ALL);

    if focused {
        block = block
            .border_type(BorderType::Thick)
            .border_style(Style::default().add_modifier(Modifier::BOLD));
    }

    if let Some(count) = word_count {
        block = block.title_bottom(Line::from(format!(" {count} words ")).right_aligned());
    }

    block
}

pub(crate) fn panel_title(title: &str, focused: bool) -> String {
    if focused {
        format!(" >> {title} ")
    } else {
        format!(" {title} ")
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
    scroll: u16,
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

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [row] = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .areas(row);
    cell
}

pub(crate) fn centered_rect_fixed_height(percent_x: u16, height: u16, area: Rect) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .areas(row);
    cell
}
