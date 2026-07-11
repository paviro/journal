use crate::AppResult;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

use crate::tui::{
    app::{App, EditLocationFocus, EditMetadataFocus, Focus, Mode, entry_view_is_available},
    editor_state::EditorPrompt,
    image::image_for_digit,
    render,
    render::insights::InsightsTab,
    state::Overlay,
};

use super::action::Action;

pub(crate) fn handle_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<bool> {
    // While the internal editor is open (and no metadata dialog is layered over
    // it), keystrokes go to the textarea — only a few control keys are intercepted
    // — bypassing the char-only Action enum so typing `q`, `/`, `n`, etc. inserts
    // literally. When a dialog is open, fall through so its handler runs.
    if app.editor.is_some() && matches!(app.overlay, Overlay::None) {
        return handle_editor_key(terminal, app, key);
    }

    let entry_view_available = entry_view_is_available(terminal.size()?.width);

    if let Some(action) = key_to_action(app, key, entry_view_available) {
        super::dispatch_action(terminal, app, action)
    } else {
        Ok(false)
    }
}

/// Translate a keystroke while the internal editor is open. Text insertion still
/// goes through dispatch as an editor input action so keyboard and mouse cannot
/// grow separate mutation paths.
fn handle_editor_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    key: KeyEvent,
) -> AppResult<bool> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    if matches!(editor_prompt(app), Some(EditorPrompt::ConfirmDiscard)) {
        let action = match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => Some(Action::EditorDiscard),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(Action::EditorClosePrompt),
            _ => None,
        };
        if let Some(action) = action {
            return super::dispatch_action(terminal, app, action);
        }
        return Ok(false);
    }

    if matches!(editor_prompt(app), Some(EditorPrompt::Help { .. })) {
        let action = match key.code {
            KeyCode::Up => Action::EditorScrollHelp(-1),
            KeyCode::Down => Action::EditorScrollHelp(1),
            KeyCode::PageUp => Action::EditorScrollHelp(-10),
            KeyCode::PageDown => Action::EditorScrollHelp(10),
            KeyCode::Home => Action::EditorScrollHelp(i16::MIN),
            KeyCode::End => Action::EditorScrollHelp(i16::MAX),
            _ => Action::EditorClosePrompt,
        };
        return super::dispatch_action(terminal, app, action);
    }

    if matches!(editor_prompt(app), Some(EditorPrompt::MetadataMenu)) {
        let action = match key.code {
            KeyCode::Char('t') => {
                Action::EditorBeginMetadata(crate::tui::state::MetadataKind::Tags)
            }
            KeyCode::Char('p') => {
                Action::EditorBeginMetadata(crate::tui::state::MetadataKind::People)
            }
            KeyCode::Char('a') => {
                Action::EditorBeginMetadata(crate::tui::state::MetadataKind::Activities)
            }
            KeyCode::Char('f') => Action::BeginEditFeelings,
            KeyCode::Char('m') => Action::BeginEditMood,
            KeyCode::Char('l') => Action::BeginEditLocation,
            _ => Action::EditorClosePrompt,
        };
        return super::dispatch_action(terminal, app, action);
    }

    match key.code {
        KeyCode::Char('s') if ctrl => {
            return super::dispatch_action(terminal, app, Action::EditorSave);
        }
        // Ctrl+A selects all, shadowing the textarea's emacs-style line-start
        // (Home still covers that).
        KeyCode::Char('a') if ctrl => {
            return super::dispatch_action(terminal, app, Action::EditorSelectAll);
        }
        // Fullscreen is on Ctrl+O, not Ctrl+F: the textarea binds Ctrl+F to
        // forward-char (emacs), which we leave to it.
        KeyCode::Char('o') if ctrl => {
            return super::dispatch_action(terminal, app, Action::EditorToggleFullscreen);
        }
        // Ctrl+G and Ctrl+T open the metadata chooser and shortcut overlay. Both
        // avoid the textarea's Ctrl bindings and Alt+letter (eaten on macOS and
        // Termux); the overlays are handled at the top of this function.
        KeyCode::Char('g') if ctrl => {
            return super::dispatch_action(terminal, app, Action::EditorOpenMetadataMenu);
        }
        KeyCode::Char('t') if ctrl => {
            return super::dispatch_action(terminal, app, Action::EditorOpenHelp);
        }
        KeyCode::Esc => {
            return super::dispatch_action(terminal, app, Action::EditorRequestDiscard);
        }
        _ => {}
    }

    super::dispatch_action(terminal, app, Action::EditorInput(key))
}

/// The open editor's current modal prompt, if an editor is open.
fn editor_prompt(app: &App) -> Option<&EditorPrompt> {
    app.editor.as_ref().map(|ed| &ed.prompt)
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
        Overlay::MetadataMenu => metadata_menu_key_to_action(key),
        Overlay::SettingsMenu => settings_menu_key_to_action(key),
        Overlay::ThemePicker(_) => theme_picker_key_to_action(key),
        Overlay::ConfirmDelete(_) => confirm_delete_key_to_action(key),
        Overlay::NewJournal(_) => new_journal_key_to_action(key),
        Overlay::EditMetadata(_) => tags_key_to_action(app, key),
        Overlay::EditFeelings(_) => feelings_key_to_action(app, key),
        Overlay::EditMood(_) => mood_key_to_action(key),
        Overlay::EditLocation(_) => location_key_to_action(app, key),
        Overlay::ImageViewer(_) => image_viewer_key_to_action(key),
        // Blocks input; it auto-resolves when the fetch lands or times out.
        Overlay::FetchingEnvironment(_) => None,
    }
}

/// Keys while the metadata reference popup is open: the listed letters open their
/// edit dialog (replacing the popup), anything else dismisses it. The letters also
/// work directly on the viewer, so this popup is only a discovery aid.
fn metadata_menu_key_to_action(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Char('t') => Action::BeginEditTags,
        KeyCode::Char('p') => Action::BeginEditPeople,
        KeyCode::Char('a') => Action::BeginEditActivities,
        KeyCode::Char('f') => Action::BeginEditFeelings,
        KeyCode::Char('m') => Action::BeginEditMood,
        KeyCode::Char('l') => Action::BeginEditLocation,
        _ => Action::CancelOverlay,
    })
}

/// Keys while the settings menu is open: `t` (its key hint) or Enter open the
/// only row — the theme picker — and anything else dismisses the menu, matching
/// the metadata menu's behavior.
fn settings_menu_key_to_action(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Char('t') | KeyCode::Enter => Action::OpenThemePicker,
        _ => Action::CancelOverlay,
    })
}

/// Keys while the theme picker is open. Esc routes to the dedicated cancel
/// action (not the generic overlay close) so the previewed theme is reverted.
fn theme_picker_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::ThemePickerCancel),
        KeyCode::Enter => Some(Action::ThemePickerConfirm),
        KeyCode::Up => Some(Action::ThemePickerMoveUp),
        KeyCode::Down => Some(Action::ThemePickerMoveDown),
        KeyCode::Char('b') => Some(Action::ThemePickerCycleChrome),
        _ => None,
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

/// Vertical-scroll keys for the focused insights list tabs, mirroring
/// [`scroll_key_to_action`] but driving the insights offset.
fn insights_scroll_key_to_action(key: KeyCode) -> Option<Action> {
    match key {
        KeyCode::Up => Some(Action::ScrollInsights(-1)),
        KeyCode::Down => Some(Action::ScrollInsights(1)),
        KeyCode::PageUp => Some(Action::PageInsights(-1)),
        KeyCode::PageDown => Some(Action::PageInsights(1)),
        KeyCode::Home => Some(Action::ScrollInsightsToStart),
        KeyCode::End => Some(Action::ScrollInsightsToEnd),
        _ => None,
    }
}

fn browse_key_to_action(app: &App, key: KeyEvent, entry_view_available: bool) -> Option<Action> {
    if app.nav.focus == Focus::EntryView
        && let Some(action) = scroll_key_to_action(key.code)
    {
        return Some(action);
    }
    // On a focused list tab, the arrow/page keys scroll the table rather than
    // moving a selection (the panel has none).
    if app.nav.focus == Focus::Insights
        && app.nav.insights_tab.is_list()
        && let Some(action) = insights_scroll_key_to_action(key.code)
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
        // Enter expands the focused insights panel to full screen; a second Enter
        // (or Esc) collapses it. Left/Right keep cycling tabs either way.
        KeyCode::Enter if app.nav.focus == Focus::Insights && !app.nav.insights_fullscreen => {
            Some(Action::ExpandInsights)
        }
        KeyCode::Enter if app.nav.focus == Focus::Insights => Some(Action::CollapseInsights),
        KeyCode::Esc if app.nav.focus == Focus::Insights && app.nav.insights_fullscreen => {
            Some(Action::CollapseInsights)
        }
        KeyCode::Enter if app.nav.focus == Focus::Journals => Some(Action::FocusRight),
        KeyCode::Enter if app.can_act_on_selected_entry() => Some(Action::ViewSelected),
        KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Char('e') if app.can_act_on_selected_entry() => Some(Action::EditSelected),
        // Toggle the insights scope while its panel is focused (its tabs switch
        // with Left/Right, handled through FocusLeft/FocusRight).
        KeyCode::Char('g') if app.nav.focus == Focus::Insights => Some(Action::ToggleInsightsScope),
        // Cycle the rolling window on the mood-driver tabs; inert elsewhere.
        KeyCode::Char('w')
            if app.nav.focus == Focus::Insights && app.nav.insights_tab.uses_timeframe() =>
        {
            Some(Action::CycleInsightsTimeframe)
        }
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
        KeyCode::Char('g')
            if key.modifiers.contains(KeyModifiers::CONTROL) && app.can_act_on_selected_entry() =>
        {
            Some(Action::OpenMetadataMenu)
        }
        KeyCode::Char('t') if app.can_act_on_selected_entry() => Some(Action::BeginEditTags),
        KeyCode::Char('p') if app.can_act_on_selected_entry() => Some(Action::BeginEditPeople),
        KeyCode::Char('a') if app.can_act_on_selected_entry() => Some(Action::BeginEditActivities),
        KeyCode::Char('f') if app.can_act_on_selected_entry() => Some(Action::BeginEditFeelings),
        KeyCode::Char('m') if app.can_act_on_selected_entry() => Some(Action::BeginEditMood),
        KeyCode::Char('l') if app.can_act_on_selected_entry() => Some(Action::BeginEditLocation),
        KeyCode::Char('s') if app.can_act_on_selected_entry() => Some(Action::ToggleStarred),
        KeyCode::Char('i' | '0'..='9')
            if app.nav.focus == Focus::EntryView && app.has_selected_entry_target() =>
        {
            image_shortcut(app, key)
        }
        KeyCode::Char('h') => Some(Action::ToggleHints),
        KeyCode::Char('j') => Some(Action::ToggleJournals),
        KeyCode::Char(',') => Some(Action::OpenSettingsMenu),
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
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::OpenMetadataMenu)
        }
        KeyCode::Char('t') => Some(Action::BeginEditTags),
        KeyCode::Char('p') => Some(Action::BeginEditPeople),
        KeyCode::Char('a') => Some(Action::BeginEditActivities),
        KeyCode::Char('f') => Some(Action::BeginEditFeelings),
        KeyCode::Char('m') => Some(Action::BeginEditMood),
        KeyCode::Char('l') => Some(Action::BeginEditLocation),
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
        KeyCode::Char('q') if app.nav.focus != Focus::Entries => Some(Action::Quit),
        // Left backs the viewer out to the results list, but is inert in multi-column
        // full screen (Esc collapses that).
        KeyCode::Left
            if app.nav.focus == Focus::EntryView
                && !(app.nav.entry_view_fullscreen && entry_view_available) =>
        {
            Some(Action::FocusLeft)
        }
        // In the search field, Right claims the key while the caret can still
        // advance, a selection is being made, or one is active (so plain Right
        // collapses it instead of leaving it painted while focus moves away);
        // only at the end of the query does it fall through to the view/focus
        // arms below.
        KeyCode::Right
            if app.nav.focus == Focus::Entries
                && (key.modifiers.contains(KeyModifiers::SHIFT)
                    || !app.search.query.cursor_at_end()
                    || app.search.query.selection_range().is_some()) =>
        {
            Some(Action::InputKey(key))
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
        KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Down => Some(Action::MoveDown),
        // Everything else typed while the search field is focused edits it —
        // including 'q', which quits only from the other panes.
        _ if app.nav.focus == Focus::Entries => Some(Action::InputKey(key)),
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
        _ => Some(Action::InputKey(key)),
    }
}

fn tags_key_to_action(app: &App, key: KeyEvent) -> Option<Action> {
    let state = app.edit_metadata_state()?;
    let focus = state.focus;
    match key.code {
        KeyCode::Esc => Some(Action::CancelOverlay),
        KeyCode::Tab => Some(Action::MetadataSwitchFocus),
        KeyCode::Enter if focus == EditMetadataFocus::List => Some(Action::MetadataSave),
        KeyCode::Enter if state.input.as_str().trim().is_empty() => Some(Action::MetadataSave),
        KeyCode::Enter => Some(Action::MetadataAddFromInput),
        KeyCode::Up if focus == EditMetadataFocus::List => Some(Action::MetadataMoveUp),
        KeyCode::Down if focus == EditMetadataFocus::List => Some(Action::MetadataMoveDown),
        KeyCode::Char(' ') if focus == EditMetadataFocus::List => Some(Action::MetadataToggle),
        _ if focus == EditMetadataFocus::Input => Some(Action::InputKey(key)),
        _ => None,
    }
}

fn feelings_key_to_action(app: &App, key: KeyEvent) -> Option<Action> {
    let focus = app.edit_feeling_state()?.focus;
    match key.code {
        KeyCode::Esc => Some(Action::CancelOverlay),
        KeyCode::Tab => Some(Action::FeelingsSwitchFocus),
        KeyCode::Enter => Some(Action::FeelingsSave),
        KeyCode::Up if focus == EditMetadataFocus::List => Some(Action::FeelingsMoveUp),
        KeyCode::Down if focus == EditMetadataFocus::List => Some(Action::FeelingsMoveDown),
        KeyCode::Right if focus == EditMetadataFocus::List => Some(Action::FeelingsExpand),
        KeyCode::Left if focus == EditMetadataFocus::List => Some(Action::FeelingsCollapse),
        KeyCode::Char(' ') if focus == EditMetadataFocus::List => Some(Action::FeelingsToggle),
        _ if focus == EditMetadataFocus::Input => Some(Action::InputKey(key)),
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

fn location_key_to_action(app: &App, key: KeyEvent) -> Option<Action> {
    let state = app.edit_location_state()?;
    let focus = state.focus;
    match key.code {
        KeyCode::Esc => Some(Action::CancelOverlay),
        KeyCode::Tab => Some(Action::LocationSwitchFocus),
        // Ctrl+L grabs the device's current location. A bare letter can't be a
        // shortcut here — the query/name fields take every plain char as text —
        // so this is matched (with the modifier) before the text-input arm.
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::LocationGrabDevice)
        }
        // Delete clears the entry's location only from the list; in the text
        // fields it forward-deletes at the caret like any editor.
        KeyCode::Delete if focus == EditLocationFocus::List => Some(Action::LocationClear),
        KeyCode::Up if focus == EditLocationFocus::List => Some(Action::LocationMoveUp),
        KeyCode::Down if focus == EditLocationFocus::List => Some(Action::LocationMoveDown),
        // On the list, Enter/Space adopt the highlighted preset or match and save.
        KeyCode::Enter | KeyCode::Char(' ') if focus == EditLocationFocus::List => {
            Some(Action::LocationSelectRow)
        }
        // In the query field, Enter looks the address/coordinates up — then, once
        // the current query is resolved, a second Enter saves instead of re-querying.
        KeyCode::Enter if focus == EditLocationFocus::Query && state.query_looked_up => {
            Some(Action::LocationSave)
        }
        KeyCode::Enter if focus == EditLocationFocus::Query => Some(Action::LocationResolve),
        // In the name field, Enter commits.
        KeyCode::Enter => Some(Action::LocationSave),
        _ if focus != EditLocationFocus::List => Some(Action::InputKey(key)),
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
        // Left steps back through the insights tabs (staying expanded if it was);
        // from the first tab it leaves the panel back to the entries column, which
        // drops full-screen so re-entering starts collapsed.
        Focus::Insights if app.nav.insights_tab.index() == 0 => {
            app.nav.insights_fullscreen = false;
            Focus::Entries
        }
        Focus::Insights => {
            app.nav.insights_tab = app.nav.insights_tab.prev();
            app.nav.scroll.reset_insights();
            Focus::Insights
        }
        Focus::EntryView => Focus::Entries,
        // When the journal list is hidden, Left stops at Entries so focus never
        // lands on a pane that isn't rendered — use `j` to bring the list back.
        Focus::Entries if app.state.ui.show_journals => Focus::Journals,
        Focus::Entries | Focus::Journals => app.nav.focus,
    };
}

pub(super) fn move_focus_right(app: &mut App, entry_view_available: bool) {
    app.nav.focus = match app.nav.focus {
        // Entering the entries column keeps whatever selection was there (none by
        // default), so the insights preview stays put until an entry is picked and
        // Right can carry on to the insights panel.
        Focus::Journals => Focus::Entries,
        Focus::Entries if entry_view_available && app.has_selected_entry_target() => {
            // Focusing the viewer lands on the preview pane; full screen is a
            // separate, explicit Enter away.
            app.nav.entry_view_fullscreen = false;
            Focus::EntryView
        }
        // With no entry to preview, the right column is the insights panel; Right
        // focuses it (landing on the first tab). Reachable at single-panel width
        // too, where it takes over the full screen.
        Focus::Entries if app.show_journal_insights_preview() => Focus::Insights,
        // Right steps forward through the tabs, stopping at the last.
        Focus::Insights => {
            if app.nav.insights_tab.index() + 1 < InsightsTab::ALL.len() {
                app.nav.insights_tab = app.nav.insights_tab.next();
                app.nav.scroll.reset_insights();
            }
            Focus::Insights
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
