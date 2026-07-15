//! Keyboard dispatch: translate a keystroke into an [`Action`] for the current
//! context (mode, focus, overlay, open editor).
//!
//! Key-selection rule, so new bindings stay consistent: a command needs a
//! modifier **only when a text field is competing for the same keystroke** in
//! that context. The two families that follow from it:
//!
//! - Command surfaces (browse list, reader, insights, list-focused dialogs) take
//!   no free text, so bare single letters are the actions (`e` edit, `d` delete,
//!   `t`/`p`/`a`/`f`/`m`/`l` metadata, `q` quit, `/` search, `?` help…).
//! - Text fields (the internal editor, the search box, dialog inputs) let bare
//!   keys type, so their commands take a modifier (`Ctrl+S`, `Ctrl+G`) or a
//!   non-text key (`Esc`, `Enter`, `Tab`).
//!
//! When a binding must span both families (the metadata menu is `Ctrl+G` in the
//! editor and browse alike) it keeps the modifier form everywhere for one muscle
//! memory, even where a bare key would be free.

use crate::AppResult;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

use crate::tui::{
    app::{AppModel, Focus, Mode, reader_is_available},
    editor_state::EditorPrompt,
    features::{location::EditLocationFocus, metadata::EditMetadataFocus},
    image::image_for_digit,
    state::{MetadataKind, Overlay},
};

use super::DispatchOutcome;
use super::action::{
    Action, BrowserAction, EditorAction, ImageAction, InsightsAction, LocationAction,
    MetadataAction, OverlayAction, ReaderAction, SearchAction, SettingsAction,
};

pub(crate) fn handle_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppModel,
    key: KeyEvent,
) -> AppResult<DispatchOutcome> {
    // While the internal editor is open (and no metadata dialog is layered over
    // it), keystrokes go to the textarea — only a few control keys are intercepted
    // — bypassing the char-only Action enum so typing `q`, `/`, `n`, etc. inserts
    // literally. When a dialog is open, fall through so its handler runs.
    if app.editor.is_some() && matches!(app.overlay, Overlay::None) {
        return handle_editor_key(terminal, app, key);
    }

    let reader_available = reader_is_available(terminal.size()?.width);

    if let Some(action) = key_to_action(app, key, reader_available) {
        super::dispatch_action(terminal, app, action)
    } else {
        Ok(DispatchOutcome::Continue)
    }
}

/// Insert a bracketed paste as one block into whichever text sink owns the caret:
/// the open editor, or a focused single-line field. Mirrors [`handle_key`]'s
/// context routing so paste and typing never grow separate mutation paths.
pub(crate) fn handle_paste<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    text: String,
) -> AppResult<DispatchOutcome> {
    if text.is_empty() {
        return Ok(DispatchOutcome::Continue);
    }
    // Only the plain typing state accepts a paste; a modal editor prompt
    // (discard-confirm, metadata chooser, help) keeps Overlay::None but must not
    // let the paste fall through into the hidden document behind it.
    let editor_typing = app
        .editor
        .as_ref()
        .is_some_and(|ed| matches!(ed.prompt, EditorPrompt::None));
    if editor_typing && matches!(app.overlay, Overlay::None) {
        return super::dispatch_action(
            terminal,
            app,
            Action::Editor(EditorAction::InsertText(text)),
        );
    }
    if app.focused_text_input_mut().is_some() {
        app.handle_text_input_paste(&text);
    }
    Ok(DispatchOutcome::Continue)
}

/// Translate a keystroke while the internal editor is open. Text insertion still
/// goes through dispatch as an editor input action so keyboard and mouse cannot
/// grow separate mutation paths.
fn handle_editor_key(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppModel,
    key: KeyEvent,
) -> AppResult<DispatchOutcome> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Clipboard ops accept either Ctrl or Cmd (Super), so `Cmd+C/X/V` behave
    // exactly like `^C/^X/^V`. Super is only reported when the terminal supports
    // the keyboard-enhancement protocol (see `runtime::terminal`); elsewhere it
    // stays Ctrl-only with no regression.
    let clip = ctrl || key.modifiers.contains(KeyModifiers::SUPER);

    if let Some(EditorPrompt::ConfirmDiscard { discard_selected }) = editor_prompt(app) {
        let selected = *discard_selected;
        let action = match key.code {
            KeyCode::Char('y' | 'Y') => Some(Action::Editor(EditorAction::Discard)),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                Some(Action::Editor(EditorAction::ClosePrompt))
            }
            KeyCode::Left => Some(Action::Overlay(OverlayAction::ConfirmSelect(true))),
            KeyCode::Right => Some(Action::Overlay(OverlayAction::ConfirmSelect(false))),
            KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::BackTab => {
                Some(Action::Overlay(OverlayAction::ConfirmSelect(!selected)))
            }
            KeyCode::Enter if selected => Some(Action::Editor(EditorAction::Discard)),
            KeyCode::Enter => Some(Action::Editor(EditorAction::ClosePrompt)),
            _ => None,
        };
        if let Some(action) = action {
            return super::dispatch_action(terminal, app, action);
        }
        return Ok(DispatchOutcome::Continue);
    }

    if matches!(editor_prompt(app), Some(EditorPrompt::Help { .. })) {
        let action = match key.code {
            KeyCode::Up => Action::Editor(EditorAction::ScrollHelp(-1)),
            KeyCode::Down => Action::Editor(EditorAction::ScrollHelp(1)),
            KeyCode::PageUp => Action::Editor(EditorAction::ScrollHelp(-10)),
            KeyCode::PageDown => Action::Editor(EditorAction::ScrollHelp(10)),
            KeyCode::Home => Action::Editor(EditorAction::ScrollHelp(i16::MIN)),
            KeyCode::End => Action::Editor(EditorAction::ScrollHelp(i16::MAX)),
            _ => Action::Editor(EditorAction::ClosePrompt),
        };
        return super::dispatch_action(terminal, app, action);
    }

    if matches!(editor_prompt(app), Some(EditorPrompt::MetadataMenu)) {
        let action = match key.code {
            KeyCode::Char('t') => Action::Metadata(MetadataAction::BeginEdit(
                crate::tui::state::MetadataKind::Tags,
            )),
            KeyCode::Char('p') => Action::Metadata(MetadataAction::BeginEdit(
                crate::tui::state::MetadataKind::People,
            )),
            KeyCode::Char('a') => Action::Metadata(MetadataAction::BeginEdit(
                crate::tui::state::MetadataKind::Activities,
            )),
            KeyCode::Char('f') => Action::Metadata(MetadataAction::BeginFeelings),
            KeyCode::Char('m') => Action::Metadata(MetadataAction::BeginMood),
            KeyCode::Char('l') => Action::Location(LocationAction::BeginEdit),
            _ => Action::Editor(EditorAction::ClosePrompt),
        };
        return super::dispatch_action(terminal, app, action);
    }

    match key.code {
        // Save takes Ctrl or Cmd (Super), so `Cmd+S` works on macOS.
        KeyCode::Char('s') if clip => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::Save));
        }
        // The editor is a text field, so commands take a modifier (bare letters
        // type). Ctrl+A select-all, Ctrl/Cmd+Z undo/redo, Ctrl/Cmd+X/C cut/copy,
        // Ctrl+V paste; Ctrl+K and Ctrl+W (cut-to-line-end, delete-word) fall
        // through to the textarea. Home covers line-start; Esc discards. Cmd+A is
        // left to the terminal (it binds it to "select the whole window").
        KeyCode::Char('a') if ctrl => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::SelectAll));
        }
        // Undo/redo take Ctrl (^Z/^Y) or Cmd (Super), so macOS `Cmd+Z` undoes and
        // `Cmd+Shift+Z` redoes. A shifted `z` reaches us as uppercase `Z` or as
        // `z` with Shift held depending on the terminal's keyboard protocol, so
        // match both; Ctrl+Y stays as the emacs-ish redo alias.
        KeyCode::Char('Z') if clip => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::Redo));
        }
        KeyCode::Char('z') if clip && key.modifiers.contains(KeyModifiers::SHIFT) => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::Redo));
        }
        KeyCode::Char('z') if clip => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::Undo));
        }
        KeyCode::Char('y') if ctrl => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::Redo));
        }
        KeyCode::Char('x') if clip => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::Cut));
        }
        KeyCode::Char('c') if clip => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::Copy));
        }
        // Ctrl+V pastes the system clipboard (native read on desktop, internal yank
        // as the fallback). `Cmd+V` isn't bound here: the terminal owns it as a paste
        // gesture, delivering the same system clipboard as a bracketed paste — so
        // both routes land the one clipboard, not two competing pastes.
        KeyCode::Char('v') if ctrl => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::Paste));
        }
        // Fullscreen is on Ctrl+O, not Ctrl+F: the textarea binds Ctrl+F to
        // forward-char (emacs), which we leave to it.
        KeyCode::Char('o') if ctrl => {
            return super::dispatch_action(
                terminal,
                app,
                Action::Editor(EditorAction::ToggleFullscreen),
            );
        }
        // Ctrl+G and Ctrl+T open the metadata chooser and shortcut overlay. Both
        // avoid the textarea's Ctrl bindings and Alt+letter (eaten on macOS and
        // Termux); the overlays are handled at the top of this function.
        KeyCode::Char('g') if ctrl => {
            return super::dispatch_action(
                terminal,
                app,
                Action::Editor(EditorAction::OpenMetadataMenu),
            );
        }
        KeyCode::Char('t') if ctrl => {
            return super::dispatch_action(terminal, app, Action::Editor(EditorAction::OpenHelp));
        }
        KeyCode::Esc => {
            return super::dispatch_action(
                terminal,
                app,
                Action::Editor(EditorAction::RequestDiscard),
            );
        }
        _ => {}
    }

    // Rewrite macOS Option/Cmd navigation chords the textarea doesn't bind into
    // the equivalent key it does, then feed that through the same Input path.
    let key = macos_nav_alias(key).unwrap_or(key);
    super::dispatch_action(terminal, app, Action::Editor(EditorAction::Input(key)))
}

/// Alias the macOS Option/Cmd editing chords the textarea leaves unbound onto the
/// key it already understands, preserving Shift so the move extends the selection.
/// Returns `None` for anything already bound (or needing no alias). Option (Alt)
/// chords are ESC-prefixed and always delivered. Cmd+←/→/↑/↓ are intentionally
/// *not* aliased: terminals bind those to their own window/scroll navigation, so
/// they never reach the app (Home/End and Ctrl+↑/↓ cover the same moves).
fn macos_nav_alias(key: KeyEvent) -> Option<KeyEvent> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let sup = key.modifiers.contains(KeyModifiers::SUPER);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let with = |code: KeyCode, mods: KeyModifiers| {
        let mods = if shift {
            mods | KeyModifiers::SHIFT
        } else {
            mods
        };
        Some(KeyEvent::new(code, mods))
    };
    match key.code {
        // Option+←/→: word back/forward — the textarea binds Ctrl+←/→ for that.
        KeyCode::Left if alt && !ctrl && !sup => with(KeyCode::Left, KeyModifiers::CONTROL),
        KeyCode::Right if alt && !ctrl && !sup => with(KeyCode::Right, KeyModifiers::CONTROL),
        _ => None,
    }
}

/// The open editor's current modal prompt, if an editor is open.
fn editor_prompt(app: &AppModel) -> Option<&EditorPrompt> {
    app.editor.as_ref().map(|ed| &ed.prompt)
}

pub(super) fn key_to_action(
    app: &AppModel,
    key: KeyEvent,
    reader_available: bool,
) -> Option<Action> {
    match &app.overlay {
        Overlay::None if app.nav.mode == Mode::Search => {
            search_key_to_action(app, key, reader_available)
        }
        Overlay::None => browse_key_to_action(app, key, reader_available),
        Overlay::MetadataMenu => metadata_menu_key_to_action(key),
        Overlay::SettingsMenu => settings_menu_key_to_action(key),
        Overlay::Help { .. } => help_key_to_action(key),
        Overlay::ThemePicker(_) => theme_picker_key_to_action(key),
        Overlay::ConfirmDelete(_, selected) => confirm_delete_key_to_action(key, *selected),
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
        KeyCode::Char('t') => Action::Metadata(MetadataAction::BeginEdit(MetadataKind::Tags)),
        KeyCode::Char('p') => Action::Metadata(MetadataAction::BeginEdit(MetadataKind::People)),
        KeyCode::Char('a') => Action::Metadata(MetadataAction::BeginEdit(MetadataKind::Activities)),
        KeyCode::Char('f') => Action::Metadata(MetadataAction::BeginFeelings),
        KeyCode::Char('m') => Action::Metadata(MetadataAction::BeginMood),
        KeyCode::Char('l') => Action::Location(LocationAction::BeginEdit),
        _ => Action::Overlay(OverlayAction::Cancel),
    })
}

/// Keys while the settings menu is open: `t` (its key hint) or Enter open the
/// only row — the theme picker — and anything else dismisses the menu, matching
/// the metadata menu's behavior.
fn settings_menu_key_to_action(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Char('t') | KeyCode::Enter => Action::Settings(SettingsAction::OpenThemePicker),
        _ => Action::Overlay(OverlayAction::Cancel),
    })
}

/// Keys while the global help cheatsheet is open: the arrow/page/home/end keys
/// scroll it, and anything else closes it — it is a reference, not interactive.
fn help_key_to_action(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Up => Action::Overlay(OverlayAction::HelpScroll(-1)),
        KeyCode::Down => Action::Overlay(OverlayAction::HelpScroll(1)),
        KeyCode::PageUp => Action::Overlay(OverlayAction::HelpScroll(-10)),
        KeyCode::PageDown => Action::Overlay(OverlayAction::HelpScroll(10)),
        KeyCode::Home => Action::Overlay(OverlayAction::HelpScroll(i16::MIN)),
        KeyCode::End => Action::Overlay(OverlayAction::HelpScroll(i16::MAX)),
        _ => Action::Overlay(OverlayAction::Cancel),
    })
}

/// Keys while the theme picker is open. Esc routes to the dedicated cancel
/// action (not the generic overlay close) so the previewed theme is reverted.
fn theme_picker_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::Settings(SettingsAction::ThemePickerCancel)),
        KeyCode::Enter => Some(Action::Settings(SettingsAction::ThemePickerConfirm)),
        KeyCode::Up => Some(Action::Metadata(MetadataAction::MoveSelection(-1))),
        KeyCode::Down => Some(Action::Metadata(MetadataAction::MoveSelection(1))),
        KeyCode::Char('b') => Some(Action::Settings(SettingsAction::ThemePickerCycleChrome)),
        KeyCode::Char('m') => Some(Action::Settings(SettingsAction::ThemePickerCycleMode)),
        KeyCode::Tab => Some(Action::Settings(SettingsAction::ThemePickerToggleScope)),
        _ => None,
    }
}

/// Map a digit key to the image index it opens (`0`–`9`), gated on that image
/// existing. Shared by browse and the search entry view.
fn image_shortcut(app: &AppModel, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('i') if app.selected_entry_image_count() > 0 => {
            Some(Action::Images(ImageAction::OpenViewer(0)))
        }
        KeyCode::Char(ch) => {
            let index = image_for_digit(ch)?;
            (index < app.selected_entry_image_count())
                .then_some(Action::Images(ImageAction::OpenViewer(index)))
        }
        _ => None,
    }
}

fn scroll_key_to_action(key: KeyCode) -> Option<Action> {
    match key {
        KeyCode::Up => Some(Action::Reader(ReaderAction::ScrollLines(-1))),
        KeyCode::Down => Some(Action::Reader(ReaderAction::ScrollLines(1))),
        KeyCode::PageUp => Some(Action::Reader(ReaderAction::ScrollPages(-1))),
        KeyCode::PageDown => Some(Action::Reader(ReaderAction::ScrollPages(1))),
        KeyCode::Home => Some(Action::Reader(ReaderAction::ScrollToStart)),
        KeyCode::End => Some(Action::Reader(ReaderAction::ScrollToEnd)),
        _ => None,
    }
}

/// Vertical-scroll keys for the focused insights list tabs, mirroring
/// [`scroll_key_to_action`] but driving the insights offset.
fn insights_scroll_key_to_action(key: KeyCode) -> Option<Action> {
    match key {
        KeyCode::Up => Some(Action::Insights(InsightsAction::ScrollLines(-1))),
        KeyCode::Down => Some(Action::Insights(InsightsAction::ScrollLines(1))),
        KeyCode::PageUp => Some(Action::Insights(InsightsAction::ScrollPages(-1))),
        KeyCode::PageDown => Some(Action::Insights(InsightsAction::ScrollPages(1))),
        KeyCode::Home => Some(Action::Insights(InsightsAction::ScrollToStart)),
        KeyCode::End => Some(Action::Insights(InsightsAction::ScrollToEnd)),
        _ => None,
    }
}

fn browse_key_to_action(app: &AppModel, key: KeyEvent, reader_available: bool) -> Option<Action> {
    if app.nav.focus == Focus::Reader
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
        KeyCode::Char('r') => Some(Action::RefreshLibrary),
        // Search where its scope is clear: the journals column (all) and the
        // entries column (this journal).
        KeyCode::Char('/') if matches!(app.nav.focus, Focus::Journals | Focus::Entries) => {
            Some(Action::Search(SearchAction::Begin))
        }
        // Left backs out one level, but does nothing in multi-column full screen —
        // there, Esc collapses back to the focused reader pane instead.
        KeyCode::Left
            if !(app.nav.focus == Focus::Reader
                && app.nav.reader_fullscreen
                && reader_available) =>
        {
            Some(Action::Browser(BrowserAction::FocusLeft))
        }
        KeyCode::Right
            if app.nav.focus == Focus::Entries
                && !reader_available
                && app.has_selected_entry_target() =>
        {
            Some(Action::Browser(BrowserAction::ViewSelected))
        }
        KeyCode::Right => Some(Action::Browser(BrowserAction::FocusRight)),
        // Second Enter on the focused viewer expands it to full screen (multi-column
        // only; single-column already renders it full screen).
        KeyCode::Enter
            if app.nav.focus == Focus::Reader && reader_available && !app.nav.reader_fullscreen =>
        {
            Some(Action::Reader(ReaderAction::SetFullscreen(true)))
        }
        // Enter again closes the full-screen viewer: back to the focused pane in
        // multi-column, or out to the entries column in single-column.
        KeyCode::Enter if app.nav.focus == Focus::Reader && app.nav.reader_fullscreen => {
            Some(Action::Reader(ReaderAction::SetFullscreen(false)))
        }
        KeyCode::Enter if app.nav.focus == Focus::Reader => {
            Some(Action::Browser(BrowserAction::FocusLeft))
        }
        // Esc collapses full screen back to the focused pane; otherwise it exits the
        // viewer to the entries column.
        KeyCode::Esc if app.nav.focus == Focus::Reader && app.nav.reader_fullscreen => {
            Some(Action::Reader(ReaderAction::SetFullscreen(false)))
        }
        KeyCode::Esc if app.nav.focus == Focus::Reader => {
            Some(Action::Browser(BrowserAction::FocusLeft))
        }
        // Enter expands the focused insights panel to full screen; a second Enter
        // (or Esc) collapses it. Left/Right keep cycling tabs either way.
        KeyCode::Enter if app.nav.focus == Focus::Insights && !app.nav.insights_fullscreen => {
            Some(Action::Insights(InsightsAction::SetFullscreen(true)))
        }
        KeyCode::Enter if app.nav.focus == Focus::Insights => {
            Some(Action::Insights(InsightsAction::SetFullscreen(false)))
        }
        KeyCode::Esc if app.nav.focus == Focus::Insights && app.nav.insights_fullscreen => {
            Some(Action::Insights(InsightsAction::SetFullscreen(false)))
        }
        KeyCode::Enter if app.nav.focus == Focus::Journals => {
            Some(Action::Browser(BrowserAction::FocusRight))
        }
        KeyCode::Enter if app.can_act_on_selected_entry() => {
            Some(Action::Browser(BrowserAction::ViewSelected))
        }
        KeyCode::Up => Some(Action::Browser(BrowserAction::MoveSelection(-1))),
        KeyCode::Down => Some(Action::Browser(BrowserAction::MoveSelection(1))),
        KeyCode::Char('e') if app.can_act_on_selected_entry() => {
            Some(Action::Browser(BrowserAction::EditSelected))
        }
        // Toggle the insights scope while its panel is focused (its tabs switch
        // with Left/Right, handled through FocusLeft/FocusRight).
        KeyCode::Char('g') if app.nav.focus == Focus::Insights => {
            Some(Action::Insights(InsightsAction::ToggleScope))
        }
        // Cycle the rolling window on the mood-driver tabs; inert elsewhere.
        KeyCode::Char('w')
            if app.nav.focus == Focus::Insights && app.nav.insights_tab.uses_timeframe() =>
        {
            Some(Action::Insights(InsightsAction::CycleTimeframe))
        }
        KeyCode::Char('n') if app.nav.focus == Focus::Journals => {
            Some(Action::Settings(SettingsAction::NewJournal))
        }
        KeyCode::Char('n') => Some(Action::Browser(BrowserAction::NewEntry)),
        KeyCode::Char('d')
            if app.nav.focus == Focus::Journals && app.selected_journal().is_some() =>
        {
            Some(Action::Browser(BrowserAction::BeginDelete))
        }
        KeyCode::Char('d') if app.can_act_on_selected_entry() => {
            Some(Action::Browser(BrowserAction::BeginDelete))
        }
        KeyCode::Char('a')
            if app.nav.focus == Focus::Journals && app.selected_journal().is_some() =>
        {
            Some(Action::Settings(SettingsAction::ToggleArchiveJournal))
        }
        KeyCode::Char('g')
            if key.modifiers.contains(KeyModifiers::CONTROL) && app.can_act_on_selected_entry() =>
        {
            Some(Action::Metadata(MetadataAction::OpenMenu))
        }
        KeyCode::Char('t') if app.can_act_on_selected_entry() => Some(Action::Metadata(
            MetadataAction::BeginEdit(MetadataKind::Tags),
        )),
        KeyCode::Char('p') if app.can_act_on_selected_entry() => Some(Action::Metadata(
            MetadataAction::BeginEdit(MetadataKind::People),
        )),
        KeyCode::Char('a') if app.can_act_on_selected_entry() => Some(Action::Metadata(
            MetadataAction::BeginEdit(MetadataKind::Activities),
        )),
        KeyCode::Char('f') if app.can_act_on_selected_entry() => {
            Some(Action::Metadata(MetadataAction::BeginFeelings))
        }
        KeyCode::Char('m') if app.can_act_on_selected_entry() => {
            Some(Action::Metadata(MetadataAction::BeginMood))
        }
        KeyCode::Char('l') if app.can_act_on_selected_entry() => {
            Some(Action::Location(LocationAction::BeginEdit))
        }
        KeyCode::Char('s') if app.can_act_on_selected_entry() => {
            Some(Action::Browser(BrowserAction::ToggleStarred))
        }
        // Images open from the reader or the entries list.
        KeyCode::Char('i' | '0'..='9')
            if matches!(app.nav.focus, Focus::Reader | Focus::Entries)
                && app.has_selected_entry_target() =>
        {
            image_shortcut(app, key)
        }
        KeyCode::Char('h') => Some(Action::Overlay(OverlayAction::ToggleHints)),
        KeyCode::Char('j') => Some(Action::Overlay(OverlayAction::ToggleJournals)),
        KeyCode::Char(',') => Some(Action::Settings(SettingsAction::OpenMenu)),
        KeyCode::Char('?') => Some(Action::Overlay(OverlayAction::OpenHelp)),
        _ => None,
    }
}

/// Actions available on the focused entry view when it holds an actionable
/// target: edit, delete, the metadata/mood editors, and image shortcuts. Callers
/// apply the shared focus+target guard once rather than on every key.
fn reader_key_to_action(app: &AppModel, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('e') => Some(Action::Browser(BrowserAction::EditSelected)),
        KeyCode::Char('d') => Some(Action::Browser(BrowserAction::BeginDelete)),
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::Metadata(MetadataAction::OpenMenu))
        }
        KeyCode::Char('t') => Some(Action::Metadata(MetadataAction::BeginEdit(
            MetadataKind::Tags,
        ))),
        KeyCode::Char('p') => Some(Action::Metadata(MetadataAction::BeginEdit(
            MetadataKind::People,
        ))),
        KeyCode::Char('a') => Some(Action::Metadata(MetadataAction::BeginEdit(
            MetadataKind::Activities,
        ))),
        KeyCode::Char('f') => Some(Action::Metadata(MetadataAction::BeginFeelings)),
        KeyCode::Char('m') => Some(Action::Metadata(MetadataAction::BeginMood)),
        KeyCode::Char('l') => Some(Action::Location(LocationAction::BeginEdit)),
        KeyCode::Char('s') => Some(Action::Browser(BrowserAction::ToggleStarred)),
        KeyCode::Char('i' | '0'..='9') => image_shortcut(app, key),
        _ => None,
    }
}

fn search_key_to_action(app: &AppModel, key: KeyEvent, reader_available: bool) -> Option<Action> {
    if app.nav.focus == Focus::Reader {
        if let Some(action) = scroll_key_to_action(key.code) {
            return Some(action);
        }
        if app.has_selected_entry_target()
            && let Some(action) = reader_key_to_action(app, key)
        {
            return Some(action);
        }
    }
    match key.code {
        // Second Enter on the focused viewer expands it to full screen (multi-column).
        KeyCode::Enter
            if app.nav.focus == Focus::Reader && reader_available && !app.nav.reader_fullscreen =>
        {
            Some(Action::Reader(ReaderAction::SetFullscreen(true)))
        }
        // Enter again closes the full-screen viewer (collapse in multi-column, or
        // back to the results list in single-column).
        KeyCode::Enter if app.nav.focus == Focus::Reader && app.nav.reader_fullscreen => {
            Some(Action::Reader(ReaderAction::SetFullscreen(false)))
        }
        KeyCode::Enter if app.nav.focus == Focus::Reader => {
            Some(Action::Browser(BrowserAction::FocusLeft))
        }
        // Esc collapses full screen back to the focused pane before it exits search.
        KeyCode::Esc if app.nav.focus == Focus::Reader && app.nav.reader_fullscreen => {
            Some(Action::Reader(ReaderAction::SetFullscreen(false)))
        }
        KeyCode::Esc => Some(Action::Search(SearchAction::Exit)),
        KeyCode::Char('q') if app.nav.focus != Focus::Entries => Some(Action::Quit),
        // `?` opens the cheatsheet from the panes, but types into the search field.
        KeyCode::Char('?') if app.nav.focus != Focus::Entries => {
            Some(Action::Overlay(OverlayAction::OpenHelp))
        }
        // Left backs the viewer out to the results list, but is inert in multi-column
        // full screen (Esc collapses that).
        KeyCode::Left
            if app.nav.focus == Focus::Reader
                && !(app.nav.reader_fullscreen && reader_available) =>
        {
            Some(Action::Browser(BrowserAction::FocusLeft))
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
            Some(Action::Overlay(OverlayAction::InputKey(key)))
        }
        KeyCode::Right
            if app.nav.focus == Focus::Entries
                && !reader_available
                && app.has_selected_entry_target() =>
        {
            Some(Action::Browser(BrowserAction::ViewSelected))
        }
        KeyCode::Right if app.nav.focus == Focus::Entries && reader_available => {
            Some(Action::Browser(BrowserAction::FocusRight))
        }
        KeyCode::Enter if app.can_act_on_selected_entry() => {
            Some(Action::Browser(BrowserAction::ViewSelected))
        }
        KeyCode::Up => Some(Action::Browser(BrowserAction::MoveSelection(-1))),
        KeyCode::Down => Some(Action::Browser(BrowserAction::MoveSelection(1))),
        // Everything else typed while the search field is focused edits it —
        // including 'q', which quits only from the other panes.
        _ if app.nav.focus == Focus::Entries => Some(Action::Overlay(OverlayAction::InputKey(key))),
        _ => None,
    }
}

/// `selected` is the highlighted button (`true` = Delete): Enter commits it,
/// arrows move it, and the `y`/`n` shortcuts still fire directly.
fn confirm_delete_key_to_action(key: KeyEvent, selected: bool) -> Option<Action> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            Some(Action::Browser(BrowserAction::ConfirmDelete))
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            Some(Action::Overlay(OverlayAction::Cancel))
        }
        KeyCode::Left => Some(Action::Overlay(OverlayAction::ConfirmSelect(true))),
        KeyCode::Right => Some(Action::Overlay(OverlayAction::ConfirmSelect(false))),
        KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::BackTab => {
            Some(Action::Overlay(OverlayAction::ConfirmSelect(!selected)))
        }
        KeyCode::Enter if selected => Some(Action::Browser(BrowserAction::ConfirmDelete)),
        KeyCode::Enter => Some(Action::Overlay(OverlayAction::Cancel)),
        _ => None,
    }
}

fn new_journal_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::Overlay(OverlayAction::Cancel)),
        KeyCode::Enter => Some(Action::Settings(SettingsAction::JournalInputSubmit)),
        _ => Some(Action::Overlay(OverlayAction::InputKey(key))),
    }
}

fn tags_key_to_action(app: &AppModel, key: KeyEvent) -> Option<Action> {
    let state = app.edit_metadata_state()?;
    let focus = state.focus;
    match key.code {
        KeyCode::Esc => Some(Action::Overlay(OverlayAction::Cancel)),
        KeyCode::Tab => Some(Action::Metadata(MetadataAction::SwitchFocus)),
        KeyCode::Enter if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::Save))
        }
        KeyCode::Enter if state.input.as_str().trim().is_empty() => {
            Some(Action::Metadata(MetadataAction::Save))
        }
        KeyCode::Enter => Some(Action::Metadata(MetadataAction::AddFromInput)),
        KeyCode::Up if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::MoveSelection(-1)))
        }
        KeyCode::Down if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::MoveSelection(1)))
        }
        KeyCode::Char(' ') if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::Toggle))
        }
        _ if focus == EditMetadataFocus::Input => {
            Some(Action::Overlay(OverlayAction::InputKey(key)))
        }
        _ => None,
    }
}

fn feelings_key_to_action(app: &AppModel, key: KeyEvent) -> Option<Action> {
    let focus = app.edit_feeling_state()?.focus;
    match key.code {
        KeyCode::Esc => Some(Action::Overlay(OverlayAction::Cancel)),
        KeyCode::Tab => Some(Action::Metadata(MetadataAction::FeelingsSwitchFocus)),
        KeyCode::Enter => Some(Action::Metadata(MetadataAction::FeelingsSave)),
        KeyCode::Up if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::MoveSelection(-1)))
        }
        KeyCode::Down if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::MoveSelection(1)))
        }
        KeyCode::Right if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::FeelingsExpand))
        }
        KeyCode::Left if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::FeelingsCollapse))
        }
        KeyCode::Char(' ') if focus == EditMetadataFocus::List => {
            Some(Action::Metadata(MetadataAction::FeelingsToggle))
        }
        _ if focus == EditMetadataFocus::Input => {
            Some(Action::Overlay(OverlayAction::InputKey(key)))
        }
        _ => None,
    }
}

fn mood_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::Overlay(OverlayAction::Cancel)),
        KeyCode::Enter => Some(Action::Metadata(MetadataAction::MoodSave)),
        KeyCode::Delete | KeyCode::Backspace => Some(Action::Metadata(MetadataAction::MoodClear)),
        KeyCode::Left => Some(Action::Metadata(MetadataAction::AdjustMood(-1))),
        KeyCode::Right => Some(Action::Metadata(MetadataAction::AdjustMood(1))),
        _ => None,
    }
}

fn location_key_to_action(app: &AppModel, key: KeyEvent) -> Option<Action> {
    let state = app.edit_location_state()?;
    let focus = state.focus;
    match key.code {
        KeyCode::Esc => Some(Action::Overlay(OverlayAction::Cancel)),
        KeyCode::Tab => Some(Action::Location(LocationAction::SwitchFocus)),
        // Ctrl+L grabs the device's current location. A bare letter can't be a
        // shortcut here — the query/name fields take every plain char as text —
        // so this is matched (with the modifier) before the text-input arm.
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::Location(LocationAction::GrabDevice))
        }
        // Delete clears the entry's location only from the list; in the text
        // fields it forward-deletes at the caret like any editor.
        KeyCode::Delete if focus == EditLocationFocus::List => {
            Some(Action::Location(LocationAction::Clear))
        }
        KeyCode::Up if focus == EditLocationFocus::List => {
            Some(Action::Metadata(MetadataAction::MoveSelection(-1)))
        }
        KeyCode::Down if focus == EditLocationFocus::List => {
            Some(Action::Metadata(MetadataAction::MoveSelection(1)))
        }
        // On the list, Enter/Space adopt the highlighted preset or match and save.
        KeyCode::Enter | KeyCode::Char(' ') if focus == EditLocationFocus::List => {
            Some(Action::Location(LocationAction::SelectRow))
        }
        // In the query field, Enter looks the address/coordinates up — then, once
        // the current query is resolved, a second Enter saves instead of re-querying.
        KeyCode::Enter if focus == EditLocationFocus::Query && state.query_looked_up => {
            Some(Action::Location(LocationAction::Save))
        }
        KeyCode::Enter if focus == EditLocationFocus::Query => {
            Some(Action::Location(LocationAction::Resolve))
        }
        // In the name field, Enter commits.
        KeyCode::Enter => Some(Action::Location(LocationAction::Save)),
        _ if focus != EditLocationFocus::List => {
            Some(Action::Overlay(OverlayAction::InputKey(key)))
        }
        _ => None,
    }
}

fn image_viewer_key_to_action(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('i') => {
            Some(Action::Overlay(OverlayAction::Cancel))
        }
        KeyCode::Left | KeyCode::Up => Some(Action::Images(ImageAction::StepViewer(-1))),
        KeyCode::Right | KeyCode::Down => Some(Action::Images(ImageAction::StepViewer(1))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn option_arrows_alias_to_word_nav() {
        assert_eq!(
            macos_nav_alias(ev(KeyCode::Left, KeyModifiers::ALT)),
            Some(ev(KeyCode::Left, KeyModifiers::CONTROL))
        );
        assert_eq!(
            macos_nav_alias(ev(KeyCode::Right, KeyModifiers::ALT)),
            Some(ev(KeyCode::Right, KeyModifiers::CONTROL))
        );
        // Shift is preserved so Option+Shift+arrow extends the selection.
        assert_eq!(
            macos_nav_alias(ev(KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT)),
            Some(ev(
                KeyCode::Right,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ))
        );
    }

    #[test]
    fn cmd_arrows_are_not_aliased() {
        // Terminals eat Cmd+arrows for their own navigation, so we don't rewrite
        // them (Home/End and Ctrl+↑/↓ cover the same moves).
        for code in [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down] {
            assert_eq!(macos_nav_alias(ev(code, KeyModifiers::SUPER)), None);
        }
    }

    #[test]
    fn already_bound_or_plain_keys_are_left_alone() {
        // Plain and Ctrl arrows the textarea already handles are not rewritten.
        assert_eq!(macos_nav_alias(ev(KeyCode::Left, KeyModifiers::NONE)), None);
        assert_eq!(
            macos_nav_alias(ev(KeyCode::Left, KeyModifiers::CONTROL)),
            None
        );
        // Ctrl+Alt+Left is the textarea's line-head; don't hijack it for word-back.
        assert_eq!(
            macos_nav_alias(ev(KeyCode::Left, KeyModifiers::ALT | KeyModifiers::CONTROL)),
            None
        );
    }
}
