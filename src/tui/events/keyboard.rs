use crate::AppResult;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, entry_view_is_available},
    image::image_for_digit,
    render,
    state::{EditMetadataFocus, Overlay},
};

use super::action::Action;

pub(crate) fn handle_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<bool> {
    let entry_view_available = entry_view_is_available(terminal.size()?.width);

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
        Overlay::None if app.nav.mode == Mode::Search => {
            search_key_to_action(app, key, entry_view_available)
        }
        Overlay::None => browse_key_to_action(app, key, entry_view_available),
        Overlay::ConfirmDelete(_) => confirm_delete_key_to_action(key),
        Overlay::NewJournal(_) => new_journal_key_to_action(key),
        Overlay::EditMetadata(_) => tags_key_to_action(app, key),
        Overlay::EditFeelings(_) => feelings_key_to_action(key),
        Overlay::EditMood(_) => mood_key_to_action(key),
        Overlay::ImageViewer(_) => image_viewer_key_to_action(key),
    }
}

/// Map a digit key to the image index it opens (`0`–`9`), gated on that image
/// existing. Shared by browse and the search entry view.
fn image_shortcut(app: &App, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('i') if app.selected_entry_image_count() > 0 => {
            Some(Action::OpenImageViewer(0))
        }
        KeyCode::Char(ch) => {
            let index = image_for_digit(ch)?;
            (index < app.selected_entry_image_count()).then_some(Action::OpenImageViewer(index))
        }
        _ => None,
    }
}

fn scroll_key_to_action(key: KeyCode) -> Option<Action> {
    match key {
        KeyCode::Up => Some(Action::ScrollEntryView(-1)),
        KeyCode::Down => Some(Action::ScrollEntryView(1)),
        KeyCode::PageUp => Some(Action::PageEntryView(-1)),
        KeyCode::PageDown => Some(Action::PageEntryView(1)),
        KeyCode::Home => Some(Action::ScrollEntryViewToStart),
        KeyCode::End => Some(Action::ScrollEntryViewToEnd),
        _ => None,
    }
}

fn browse_key_to_action(app: &App, key: KeyEvent, entry_view_available: bool) -> Option<Action> {
    if app.nav.focus == Focus::EntryView
        && let Some(action) = scroll_key_to_action(key.code)
    {
        return Some(action);
    }
    match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Char('/') => Some(Action::BeginSearch),
        // Left backs out one level, but does nothing in multi-column full screen —
        // there, Esc collapses back to the focused preview pane instead.
        KeyCode::Left
            if !(app.nav.focus == Focus::EntryView
                && app.nav.entry_view_fullscreen
                && entry_view_available) =>
        {
            Some(Action::FocusLeft)
        }
        KeyCode::Right
            if app.nav.focus == Focus::Entries
                && !entry_view_available
                && app.has_selected_entry_target() =>
        {
            Some(Action::ViewSelected)
        }
        KeyCode::Right => Some(Action::FocusRight),
        // Second Enter on the focused viewer expands it to full screen (multi-column
        // only; single-column already renders it full screen).
        KeyCode::Enter
            if app.nav.focus == Focus::EntryView
                && entry_view_available
                && !app.nav.entry_view_fullscreen =>
        {
            Some(Action::ExpandEntryView)
        }
        // Enter again closes the full-screen viewer: back to the focused pane in
        // multi-column, or out to the entries column in single-column.
        KeyCode::Enter if app.nav.focus == Focus::EntryView && app.nav.entry_view_fullscreen => {
            Some(Action::CollapseEntryView)
        }
        KeyCode::Enter if app.nav.focus == Focus::EntryView => Some(Action::FocusLeft),
        // Esc collapses full screen back to the focused pane; otherwise it exits the
        // viewer to the entries column.
        KeyCode::Esc if app.nav.focus == Focus::EntryView && app.nav.entry_view_fullscreen => {
            Some(Action::CollapseEntryView)
        }
        KeyCode::Esc if app.nav.focus == Focus::EntryView => Some(Action::FocusLeft),
        KeyCode::Enter if app.nav.focus == Focus::Journals => Some(Action::FocusRight),
        KeyCode::Enter if app.can_act_on_selected_entry() => Some(Action::ViewSelected),
        KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Char('e') if app.can_act_on_selected_entry() => Some(Action::EditSelected),
        KeyCode::Char('n') if app.nav.focus == Focus::Journals => Some(Action::NewJournal),
        KeyCode::Char('n') => Some(Action::NewEntry),
        KeyCode::Char('d')
            if app.nav.focus == Focus::Journals && app.selected_journal().is_some() =>
        {
            Some(Action::BeginDelete)
        }
        KeyCode::Char('d') if app.can_act_on_selected_entry() => Some(Action::BeginDelete),
        KeyCode::Char('a')
            if app.nav.focus == Focus::Journals && app.selected_journal().is_some() =>
        {
            Some(Action::ToggleArchiveJournal)
        }
        KeyCode::Char('t') if app.can_act_on_selected_entry() => Some(Action::BeginEditTags),
        KeyCode::Char('p') if app.can_act_on_selected_entry() => Some(Action::BeginEditPeople),
        KeyCode::Char('a') if app.can_act_on_selected_entry() => Some(Action::BeginEditActivities),
        KeyCode::Char('f') if app.can_act_on_selected_entry() => Some(Action::BeginEditFeelings),
        KeyCode::Char('m') if app.can_act_on_selected_entry() => Some(Action::BeginEditMood),
        KeyCode::Char('s') if app.can_act_on_selected_entry() => Some(Action::ToggleStarred),
        KeyCode::Char('i' | '0'..='9')
            if app.nav.focus == Focus::EntryView && app.has_selected_entry_target() =>
        {
            image_shortcut(app, key)
        }
        KeyCode::Char('h') => Some(Action::ToggleHints),
        KeyCode::Char('j') => Some(Action::ToggleJournals),
        _ => None,
    }
}

/// Actions available on the focused entry view when it holds an actionable
/// target: edit, delete, the metadata/mood editors, and image shortcuts. Callers
/// apply the shared focus+target guard once rather than on every key.
fn entry_view_key_to_action(app: &App, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('e') => Some(Action::EditSelected),
        KeyCode::Char('d') => Some(Action::BeginDelete),
        KeyCode::Char('t') => Some(Action::BeginEditTags),
        KeyCode::Char('p') => Some(Action::BeginEditPeople),
        KeyCode::Char('a') => Some(Action::BeginEditActivities),
        KeyCode::Char('f') => Some(Action::BeginEditFeelings),
        KeyCode::Char('m') => Some(Action::BeginEditMood),
        KeyCode::Char('s') => Some(Action::ToggleStarred),
        KeyCode::Char('i' | '0'..='9') => image_shortcut(app, key),
        _ => None,
    }
}

fn search_key_to_action(app: &App, key: KeyEvent, entry_view_available: bool) -> Option<Action> {
    if app.nav.focus == Focus::EntryView {
        if let Some(action) = scroll_key_to_action(key.code) {
            return Some(action);
        }
        if app.has_selected_entry_target()
            && let Some(action) = entry_view_key_to_action(app, key)
        {
            return Some(action);
        }
    }
    match key.code {
        // Second Enter on the focused viewer expands it to full screen (multi-column).
        KeyCode::Enter
            if app.nav.focus == Focus::EntryView
                && entry_view_available
                && !app.nav.entry_view_fullscreen =>
        {
            Some(Action::ExpandEntryView)
        }
        // Enter again closes the full-screen viewer (collapse in multi-column, or
        // back to the results list in single-column).
        KeyCode::Enter if app.nav.focus == Focus::EntryView && app.nav.entry_view_fullscreen => {
            Some(Action::CollapseEntryView)
        }
        KeyCode::Enter if app.nav.focus == Focus::EntryView => Some(Action::FocusLeft),
        // Esc collapses full screen back to the focused pane before it exits search.
        KeyCode::Esc if app.nav.focus == Focus::EntryView && app.nav.entry_view_fullscreen => {
            Some(Action::CollapseEntryView)
        }
        KeyCode::Esc => Some(Action::ExitSearch),
        KeyCode::Char('q') => Some(Action::Quit),
        // Left backs the viewer out to the results list, but is inert in multi-column
        // full screen (Esc collapses that).
        KeyCode::Left
            if app.nav.focus == Focus::EntryView
                && !(app.nav.entry_view_fullscreen && entry_view_available) =>
        {
            Some(Action::FocusLeft)
        }
        // In the search field, Left/Right move the caret. Right only claims the key
        // while the caret can still advance; at the end of the query it falls
        // through to the view/focus arms below.
        KeyCode::Left if app.nav.focus == Focus::Entries => Some(Action::SearchCursorLeft),
        KeyCode::Right
            if app.nav.focus == Focus::Entries
                && app.search.cursor < app.search.query.chars().count() =>
        {
            Some(Action::SearchCursorRight)
        }
        KeyCode::Right
            if app.nav.focus == Focus::Entries
                && !entry_view_available
                && app.has_selected_entry_target() =>
        {
            Some(Action::ViewSelected)
        }
        KeyCode::Right if app.nav.focus == Focus::Entries && entry_view_available => {
            Some(Action::FocusRight)
        }
        KeyCode::Enter if app.can_act_on_selected_entry() => Some(Action::ViewSelected),
        KeyCode::Backspace if app.nav.focus == Focus::Entries => Some(Action::SearchBackspace),
        KeyCode::Char(ch) if app.nav.focus == Focus::Entries => Some(Action::SearchInput(ch)),
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
    let state = app.edit_metadata_state()?;
    let focus = state.focus;
    match key.code {
        KeyCode::Esc => Some(Action::CancelOverlay),
        KeyCode::Tab => Some(Action::MetadataSwitchFocus),
        KeyCode::Enter if focus == EditMetadataFocus::List => Some(Action::MetadataSave),
        KeyCode::Enter if state.input.trim().is_empty() => Some(Action::MetadataSave),
        KeyCode::Enter => Some(Action::MetadataAddFromInput),
        KeyCode::Up if focus == EditMetadataFocus::List => Some(Action::MetadataMoveUp),
        KeyCode::Down if focus == EditMetadataFocus::List => Some(Action::MetadataMoveDown),
        KeyCode::Char(' ') if focus == EditMetadataFocus::List => Some(Action::MetadataToggle),
        KeyCode::Backspace if focus == EditMetadataFocus::Input => Some(Action::MetadataBackspace),
        KeyCode::Char(ch) if focus == EditMetadataFocus::Input => Some(Action::MetadataInput(ch)),
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

fn image_viewer_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('i') => {
            Some(Action::CancelOverlay)
        }
        KeyCode::Left | KeyCode::Up => Some(Action::ImageViewerPrev),
        KeyCode::Right | KeyCode::Down => Some(Action::ImageViewerNext),
        _ => None,
    }
}

// ── Navigation helpers used by dispatch_action and tests ──────────────────────

pub(super) fn move_focus_left(app: &mut App) {
    // Leaving the viewer always drops full-screen mode so re-entering starts from
    // the focused preview pane again.
    app.nav.entry_view_fullscreen = false;
    app.nav.focus = match app.nav.focus {
        Focus::EntryView => Focus::Entries,
        // When the journal list is hidden, Left stops at Entries so focus never
        // lands on a pane that isn't rendered — use `j` to bring the list back.
        Focus::Entries if app.state.ui.show_journals => Focus::Journals,
        Focus::Entries | Focus::Journals => app.nav.focus,
    };
}

pub(super) fn move_focus_right(app: &mut App, entry_view_available: bool) {
    app.nav.focus = match app.nav.focus {
        Focus::Journals => {
            // Entering the entries column lands on an entry (when the journal has
            // any); the stats view is reached from there by scrolling up past the
            // first entry.
            if app.nav.selected_entry_index.is_none() && app.current_entry_list_len() > 0 {
                app.nav.selected_entry_index = Some(0);
            }
            Focus::Entries
        }
        // Don't open the entry view when no entry is selected (stats preview).
        Focus::Entries if entry_view_available && app.has_selected_entry_target() => {
            // Focusing the viewer lands on the preview pane; full screen is a
            // separate, explicit Enter away.
            app.nav.entry_view_fullscreen = false;
            Focus::EntryView
        }
        Focus::Entries | Focus::EntryView => app.nav.focus,
    };
}

pub(super) fn keep_selection_visible(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> AppResult<()> {
    let layout = render::tui_layout(super::terminal_area(terminal)?, app);
    if app.nav.focus == Focus::Journals && app.nav.mode == Mode::Browse {
        if let Some(area) = layout.journals {
            let (_, meta, list_area) = app.journal_rows(area.content);
            app.journal_list_ensure_visible(&meta, list_area.height);
        }
    } else if let Some(area) = layout.entries {
        let cache = app.entry_rows(area.text_width);
        app.entry_list_ensure_visible(&cache.meta, area.viewport_height);
    }
    Ok(())
}
