use crate::AppResult;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, entry_view_is_available},
    render,
    state::{EditTagFocus, Overlay},
};

use super::action::Action;

pub(crate) fn handle_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<bool> {
    let entry_view_available = entry_view_is_available(terminal.size()?.width);
    app.normalize_focus(entry_view_available);

    if let Some(action) = key_to_action(app, key, entry_view_available) {
        super::dispatch_action(terminal, app, action)
    } else {
        Ok(false)
    }
}

pub(super) fn key_to_action(
    app: &App,
    key: KeyEvent,
    entry_view_available: bool,
) -> Option<Action> {
    match &app.overlay {
        Overlay::None if app.entry_view_expanded => expanded_key_to_action(app, key),
        Overlay::None if app.mode == Mode::Search => {
            search_key_to_action(app, key, entry_view_available)
        }
        Overlay::None => browse_key_to_action(app, key, entry_view_available),
        Overlay::ConfirmDelete => confirm_delete_key_to_action(key),
        Overlay::NewJournal(_) => new_journal_key_to_action(key),
        Overlay::EditTags(_) => tags_key_to_action(app, key),
        Overlay::EditFeelings(_) => feelings_key_to_action(key),
        Overlay::EditMood(_) => mood_key_to_action(key),
    }
}

/// Shared scroll-key mapping for the entry view — used by both the expanded
/// entry handler and the normal browse/search handler when focus==EntryView.
fn scroll_key_to_action(key: KeyCode) -> Option<Action> {
    match key {
        KeyCode::Up | KeyCode::Char('k') => Some(Action::ScrollEntryView(-1)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::ScrollEntryView(1)),
        KeyCode::PageUp => Some(Action::PageEntryView(-1)),
        KeyCode::PageDown => Some(Action::PageEntryView(1)),
        KeyCode::Home => Some(Action::ScrollEntryViewToStart),
        KeyCode::End => Some(Action::ScrollEntryViewToEnd),
        _ => None,
    }
}

fn expanded_key_to_action(app: &App, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Esc | KeyCode::Enter | KeyCode::Left => Some(Action::CancelOverlay),
        KeyCode::Char('/') if app.mode == Mode::Browse => Some(Action::BeginSearch),
        KeyCode::Char('e') if app.has_selected_entry_target() => Some(Action::EditSelected),
        KeyCode::Char('n') if app.mode == Mode::Browse => Some(Action::NewEntry),
        KeyCode::Char('d') if app.has_selected_entry_target() => Some(Action::BeginDelete),
        KeyCode::Char('t') if app.has_selected_entry_target() => Some(Action::BeginEditTags),
        KeyCode::Char('f') if app.has_selected_entry_target() => Some(Action::BeginEditFeelings),
        KeyCode::Char('m') if app.has_selected_entry_target() => Some(Action::BeginEditMood),
        code => scroll_key_to_action(code),
    }
}

fn browse_key_to_action(app: &App, key: KeyEvent, entry_view_available: bool) -> Option<Action> {
    if app.focus == Focus::EntryView
        && let Some(action) = scroll_key_to_action(key.code)
    {
        return Some(action);
    }
    match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Char('/') => Some(Action::BeginSearch),
        KeyCode::Left => Some(Action::FocusLeft),
        KeyCode::Right
            if app.focus == Focus::Entries
                && !entry_view_available
                && app.has_selected_entry_target() =>
        {
            Some(Action::ViewSelected)
        }
        KeyCode::Right => Some(Action::FocusRight),
        KeyCode::Enter if app.focus == Focus::Journals => Some(Action::FocusRight),
        KeyCode::Enter if app.can_act_on_selected_entry() => Some(Action::ViewSelected),
        KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Char('e') if app.can_act_on_selected_entry() => Some(Action::EditSelected),
        KeyCode::Char('n') if app.focus == Focus::Journals => Some(Action::NewJournal),
        KeyCode::Char('n') => Some(Action::NewEntry),
        KeyCode::Char('d') if app.can_act_on_selected_entry() => Some(Action::BeginDelete),
        KeyCode::Char('t') if app.can_act_on_selected_entry() => Some(Action::BeginEditTags),
        KeyCode::Char('f') if app.can_act_on_selected_entry() => Some(Action::BeginEditFeelings),
        KeyCode::Char('m') if app.can_act_on_selected_entry() => Some(Action::BeginEditMood),
        _ => None,
    }
}

fn search_key_to_action(app: &App, key: KeyEvent, entry_view_available: bool) -> Option<Action> {
    if app.focus == Focus::EntryView
        && let Some(action) = scroll_key_to_action(key.code)
    {
        return Some(action);
    }
    match key.code {
        KeyCode::Esc => Some(Action::ExitSearch),
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Left if app.focus == Focus::EntryView => Some(Action::FocusLeft),
        KeyCode::Right
            if app.focus == Focus::Entries
                && !entry_view_available
                && app.has_selected_entry_target() =>
        {
            Some(Action::ViewSelected)
        }
        KeyCode::Right if app.focus == Focus::Entries && entry_view_available => {
            Some(Action::FocusRight)
        }
        KeyCode::Enter if app.can_act_on_selected_entry() => Some(Action::ViewSelected),
        KeyCode::Char('e') if app.focus == Focus::EntryView && app.has_selected_entry_target() => {
            Some(Action::EditSelected)
        }
        KeyCode::Char('d') if app.focus == Focus::EntryView && app.has_selected_entry_target() => {
            Some(Action::BeginDelete)
        }
        KeyCode::Char('t') if app.focus == Focus::EntryView && app.has_selected_entry_target() => {
            Some(Action::BeginEditTags)
        }
        KeyCode::Char('f') if app.focus == Focus::EntryView && app.has_selected_entry_target() => {
            Some(Action::BeginEditFeelings)
        }
        KeyCode::Char('m') if app.focus == Focus::EntryView && app.has_selected_entry_target() => {
            Some(Action::BeginEditMood)
        }
        KeyCode::Backspace if app.focus == Focus::Entries => Some(Action::SearchBackspace),
        KeyCode::Char(ch) if app.focus == Focus::Entries => Some(Action::SearchInput(ch)),
        KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Down => Some(Action::MoveDown),
        _ => None,
    }
}

fn confirm_delete_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(Action::ConfirmDelete),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(Action::CancelOverlay),
        _ => None,
    }
}

fn new_journal_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::CancelOverlay),
        KeyCode::Enter => Some(Action::JournalInputSubmit),
        KeyCode::Backspace => Some(Action::JournalInputBackspace),
        KeyCode::Char(ch) => Some(Action::JournalInputChar(ch)),
        _ => None,
    }
}

fn tags_key_to_action(app: &App, key: KeyEvent) -> Option<Action> {
    let focus = app.edit_tag_state()?.focus;
    match key.code {
        KeyCode::Esc => Some(Action::CancelOverlay),
        KeyCode::Tab => Some(Action::TagsSwitchFocus),
        KeyCode::Enter if focus == EditTagFocus::List => Some(Action::TagsSave),
        KeyCode::Enter => Some(Action::TagsAddFromInput),
        KeyCode::Up if focus == EditTagFocus::List => Some(Action::TagsMoveUp),
        KeyCode::Down if focus == EditTagFocus::List => Some(Action::TagsMoveDown),
        KeyCode::Char(' ') if focus == EditTagFocus::List => Some(Action::TagsToggle),
        KeyCode::Backspace if focus == EditTagFocus::Input => Some(Action::TagsBackspace),
        KeyCode::Char(ch) if focus == EditTagFocus::Input => Some(Action::TagsInput(ch)),
        _ => None,
    }
}

fn feelings_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::CancelOverlay),
        KeyCode::Enter => Some(Action::FeelingsSave),
        KeyCode::Up => Some(Action::FeelingsMoveUp),
        KeyCode::Down => Some(Action::FeelingsMoveDown),
        KeyCode::Char(' ') => Some(Action::FeelingsToggle),
        _ => None,
    }
}

fn mood_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::CancelOverlay),
        KeyCode::Enter => Some(Action::MoodSave),
        KeyCode::Delete | KeyCode::Backspace => Some(Action::MoodClear),
        KeyCode::Left => Some(Action::MoodDecrease),
        KeyCode::Right => Some(Action::MoodIncrease),
        _ => None,
    }
}

// ── Navigation helpers used by dispatch_action and tests ──────────────────────

pub(super) fn move_focus_left(app: &mut App) {
    app.focus = match app.focus {
        Focus::EntryView => Focus::Entries,
        Focus::Entries => Focus::Journals,
        Focus::Journals => Focus::Journals,
    };
}

pub(super) fn move_focus_right(app: &mut App, entry_view_available: bool) {
    app.focus = match app.focus {
        Focus::Journals => Focus::Entries,
        Focus::Entries if entry_view_available => Focus::EntryView,
        Focus::Entries | Focus::EntryView => app.focus,
    };
}

pub(super) fn keep_selection_visible(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let size = terminal.size()?;
    let layout = render::tui_layout(Rect::new(0, 0, size.width, size.height), app);
    if app.focus == Focus::Journals && app.mode == Mode::Browse {
        if let Some(area) = layout.journals {
            app.journal_list_ensure_visible(area.content.height);
        }
    } else if let Some(area) = layout.entries {
        let rows = render::entry_row_metadata(app, area.text_width);
        app.entry_list_ensure_visible(&rows, area.viewport_height);
    }
    Ok(())
}
