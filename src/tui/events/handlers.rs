use super::*;
use crate::tui::features::metadata::EditMetadataFocus;

/// Adjust the focused list's scroll offset so a selection moved by a handler
/// stays on screen, using the live terminal geometry.
fn keep_selection_visible<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
) -> AppResult<()> {
    let layout = render::tui_layout(super::terminal_area(terminal)?, app);
    if app.nav.focus == Focus::Journals && app.nav.mode == crate::tui::app::Mode::Browse {
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

pub(super) fn browser<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    action: BrowserAction,
) -> AppResult<Option<DispatchOutcome>> {
    match action {
        BrowserAction::FocusLeft => app.move_focus_left(),
        BrowserAction::FocusRight => {
            let available = reader_is_available(
                terminal
                    .size()
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?
                    .width,
            );
            app.move_focus_right(available);
        }
        BrowserAction::MoveSelection(delta) => {
            app.move_selection(delta);
            keep_selection_visible(terminal, app)?;
        }
        BrowserAction::EditSelected => app.open_editor_for_selected()?,
        BrowserAction::ViewSelected => view_selected(app)?,
        BrowserAction::OpenReaderLink {
            target,
            heading_line,
        } => {
            if let Some(effect) = open_reader_link(app, &target, heading_line)? {
                return Ok(Some(DispatchOutcome::Continue.with_effect(effect)));
            }
        }
        BrowserAction::BeginDelete => app.begin_confirm_delete(),
        BrowserAction::ConfirmDelete => confirm_delete(app)?,
        BrowserAction::ToggleStarred => {
            app.reload_selected_entry_from_disk()?;
            commit_entry_edit(app, toggle_starred_on_entry)?;
        }
        BrowserAction::NewEntry => app.open_editor_for_new(),
    }
    Ok(None)
}

pub(super) fn search(app: &mut AppModel, action: SearchAction) {
    match action {
        SearchAction::Begin => app.begin_search(),
        SearchAction::Exit => app.exit_search(),
    }
}

pub(super) fn editor(app: &mut AppModel, action: EditorAction) -> AppResult<()> {
    match action {
        EditorAction::Save => save_editor_with_reader_restore(app)?,
        EditorAction::RequestDiscard => request_editor_discard(app),
        EditorAction::Discard => app.cancel_editor(),
        EditorAction::ToggleFullscreen => {
            app.nav.reader_fullscreen = !app.nav.reader_fullscreen;
        }
        EditorAction::OpenMetadataMenu => set_editor_prompt(app, EditorPrompt::MetadataMenu),
        EditorAction::OpenHelp => set_editor_prompt(app, EditorPrompt::Help { scroll: 0 }),
        EditorAction::ClosePrompt => set_editor_prompt(app, EditorPrompt::None),
        EditorAction::ScrollHelp(delta) => scroll_editor_help(app, delta),
        EditorAction::Input(key) => {
            if let Some(editor) = app.editor.as_mut() {
                editor.textarea.input(key);
            }
        }
        EditorAction::InsertText(text) => {
            if let Some(editor) = app.editor.as_mut() {
                editor.textarea.insert_str(&text);
            }
        }
        EditorAction::SelectAll => {
            if let Some(editor) = app.editor.as_mut() {
                editor.textarea.select_all();
            }
        }
        EditorAction::Undo => {
            if let Some(editor) = app.editor.as_mut() {
                editor.textarea.undo();
            }
        }
        EditorAction::Redo => {
            if let Some(editor) = app.editor.as_mut() {
                editor.textarea.redo();
            }
        }
        EditorAction::Cut => {
            if let Some(editor) = app.editor.as_mut() {
                // cut() only touches the yank buffer when it removed a selection;
                // don't push a stale yank to the system clipboard otherwise.
                if editor.textarea.cut() {
                    crate::tui::clipboard::system_copy(&editor.textarea.yank_text());
                }
            }
        }
        EditorAction::Copy => {
            if let Some(editor) = app.editor.as_mut() {
                // copy() is a no-op without a selection and leaves the previous
                // yank in place, so only mirror to the system clipboard when
                // something is actually selected.
                if editor.textarea.selection_range().is_some() {
                    editor.textarea.copy();
                    crate::tui::clipboard::system_copy(&editor.textarea.yank_text());
                }
            }
        }
        EditorAction::Paste => {
            if let Some(editor) = app.editor.as_mut() {
                // Prefer the real system clipboard (desktop); fall back to the
                // internal yank where there's no native backend to read it.
                match crate::tui::clipboard::system_paste() {
                    Some(text) if !text.is_empty() => {
                        editor.textarea.insert_str(&text);
                    }
                    _ => {
                        editor.textarea.paste();
                    }
                }
            }
        }
        EditorAction::Scroll(delta) => {
            if let Some(editor) = app.editor.as_mut() {
                editor.scroll_lines(delta);
            }
        }
        EditorAction::StartSelection { col, row } => start_editor_selection(app, col, row),
        EditorAction::SelectWord { col, row } => select_editor_word(app, col, row),
        EditorAction::DragSelection { col, row } => drag_editor_selection(app, col, row),
        EditorAction::EndSelection => end_editor_selection(app),
    }
    Ok(())
}

pub(super) fn metadata<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    action: MetadataAction,
) -> AppResult<Option<DispatchOutcome>> {
    match action {
        MetadataAction::OpenMenu => {
            if app.editor.is_none() {
                app.reload_selected_entry_from_disk()?;
            }
            app.open_metadata_menu();
        }
        MetadataAction::BeginEdit(kind) => {
            set_editor_prompt(app, EditorPrompt::None);
            match kind {
                crate::tui::state::MetadataKind::Tags => app.begin_edit_tags(),
                crate::tui::state::MetadataKind::People => app.begin_edit_people(),
                crate::tui::state::MetadataKind::Activities => app.begin_edit_activities(),
            }
            reveal_open_dialog_selection(terminal, app)?;
        }
        MetadataAction::BeginFeelings => {
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_feelings();
        }
        MetadataAction::BeginMood => {
            set_editor_prompt(app, EditorPrompt::None);
            app.begin_edit_mood();
        }
        MetadataAction::MoveSelection(delta) => {
            let theme_picker = matches!(app.overlay, Overlay::ThemePicker(_));
            navigate_open_dialog(terminal, app, |list| {
                if delta < 0 {
                    list.move_up();
                } else if delta > 0 {
                    list.move_down();
                }
            })?;
            if theme_picker {
                app.theme_picker_preview();
            }
        }
        MetadataAction::Toggle => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.toggle_selected();
            }
        }
        MetadataAction::SwitchFocus => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.focus = match state.focus {
                    EditMetadataFocus::List => EditMetadataFocus::Input,
                    EditMetadataFocus::Input => EditMetadataFocus::List,
                };
            }
        }
        MetadataAction::AddFromInput => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.add_from_input();
            }
        }
        MetadataAction::Save => {
            let Some((kind, values)) = app
                .edit_metadata_state()
                .map(|state| (state.kind, state.selected.clone()))
            else {
                return Ok(None);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_metadata(kind, &values),
                |app| set_metadata_on_entry(app, kind, &values),
            )?;
        }
        MetadataAction::FeelingsToggle => {
            let height = open_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.toggle_selected();
                state.ensure_selected_visible(height);
            }
        }
        MetadataAction::FeelingsExpand => {
            let height = open_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.expand_selected();
                state.ensure_selected_visible(height);
            }
        }
        MetadataAction::FeelingsCollapse => {
            let height = open_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.collapse_selected();
                state.ensure_selected_visible(height);
            }
        }
        MetadataAction::FeelingsSwitchFocus => {
            if let Some(state) = app.edit_feeling_state_mut() {
                state.switch_focus();
            }
        }
        MetadataAction::FeelingsSave => {
            let Some(feelings) = app.edit_feeling_state().map(|state| state.selected.clone())
            else {
                return Ok(None);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_feelings(&feelings),
                |app| set_feelings_on_entry(app, &feelings),
            )?;
        }
        MetadataAction::AdjustMood(delta) => {
            if let Some(state) = app.edit_mood_state_mut() {
                state.draft = state.draft.saturating_add(delta).clamp(-5, 5);
            }
        }
        MetadataAction::MoodSave => {
            let Some(mood) = app.edit_mood_state().map(|state| state.draft) else {
                return Ok(None);
            };
            edit_or_commit(
                app,
                |app| app.set_editor_mood(Some(mood)),
                |app| set_mood_on_entry(app, Some(mood)),
            )?;
        }
        MetadataAction::MoodClear => {
            let saved = app.edit_mood_state().and_then(|state| state.saved);
            edit_or_commit(
                app,
                |app| app.set_editor_mood(None),
                |app| {
                    if saved.is_some() {
                        set_mood_on_entry(app, None)?;
                    }
                    Ok(())
                },
            )?;
        }
    }
    Ok(None)
}

pub(super) fn location(
    app: &mut AppModel,
    action: LocationAction,
) -> AppResult<Option<DispatchOutcome>> {
    match action {
        LocationAction::BeginEdit => {
            set_editor_prompt(app, EditorPrompt::None);
            if app.editor.is_none() {
                app.reload_selected_entry_from_disk()?;
            }
            app.begin_edit_location();
        }
        LocationAction::SwitchFocus => {
            if let Some(state) = app.edit_location_state_mut() {
                state.switch_focus();
            }
        }
        LocationAction::Resolve => {
            if let Some(request) = app.prepare_location_query() {
                return Ok(Some(
                    DispatchOutcome::Continue.with_effect(Effect::Geocode(request)),
                ));
            }
        }
        LocationAction::GrabDevice => {
            if let Some(request) = app.prepare_device_location() {
                return Ok(Some(
                    DispatchOutcome::Continue.with_effect(Effect::Geocode(request)),
                ));
            }
        }
        LocationAction::SelectRow => {
            if let Some(state) = app.edit_location_state_mut() {
                state.select_row();
            }
            let Some((location, zone)) = app
                .edit_location_state()
                .map(|state| (state.composed(), state.composed_timezone()))
            else {
                return Ok(None);
            };
            if let Some(request) = edit_or_commit_location(app, location, zone)? {
                return Ok(Some(
                    DispatchOutcome::Continue.with_effect(Effect::Environment(request)),
                ));
            }
        }
        LocationAction::Save => {
            let Some((location, zone)) = app
                .edit_location_state()
                .map(|state| (state.composed(), state.composed_timezone()))
            else {
                return Ok(None);
            };
            if let Some(request) = edit_or_commit_location(app, location, zone)? {
                return Ok(Some(
                    DispatchOutcome::Continue.with_effect(Effect::Environment(request)),
                ));
            }
        }
        LocationAction::Clear => {
            let _ = edit_or_commit_location(app, None, None)?;
        }
    }
    Ok(None)
}

pub(super) fn settings<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    action: SettingsAction,
) -> AppResult<()> {
    match action {
        SettingsAction::NewJournal => app.begin_new_journal_input(),
        SettingsAction::ToggleArchiveJournal => {
            toggle_archive_selected_journal(app)?;
            keep_selection_visible(terminal, app)?;
        }
        SettingsAction::JournalInputSubmit => submit_new_journal(app)?,
        SettingsAction::OpenMenu => app.open_settings_menu(),
        SettingsAction::OpenThemePicker => {
            app.open_theme_picker();
            reveal_open_dialog_selection(terminal, app)?;
        }
        SettingsAction::ThemePickerSelect(index) => app.theme_picker_select(index),
        SettingsAction::ThemePickerConfirm => app.theme_picker_confirm(),
        SettingsAction::ThemePickerCancel => app.theme_picker_cancel(),
        SettingsAction::ThemePickerCycleChrome => app.theme_picker_cycle_chrome(),
        SettingsAction::ThemePickerCycleMode => app.theme_picker_cycle_mode(),
        SettingsAction::ThemePickerToggleScope => {
            app.theme_picker_toggle_scope();
            reveal_open_dialog_selection(terminal, app)?;
        }
    }
    Ok(())
}

pub(super) fn images<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    action: ImageAction,
) -> AppResult<Option<DispatchOutcome>> {
    match action {
        ImageAction::OpenViewer(index) => {
            app.begin_image_viewer(index);
            let size = terminal
                .size()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if let Some(request) = app.prepare_image_warm(size) {
                return Ok(Some(
                    DispatchOutcome::Continue.with_effect(Effect::PrepareImages(request)),
                ));
            }
        }
        ImageAction::StepViewer(delta) => app.image_viewer_step(delta),
    }
    Ok(None)
}

pub(super) fn reader(app: &mut AppModel, action: ReaderAction) {
    match action {
        ReaderAction::ScrollLines(delta) => app.scroll_reader(delta),
        ReaderAction::ScrollPages(delta) => app.page_reader(delta),
        ReaderAction::ScrollToStart => app.nav.scroll.reader = 0,
        ReaderAction::ScrollToEnd => app.nav.scroll.reader = u16::MAX,
        ReaderAction::SetFullscreen(fullscreen) => app.nav.reader_fullscreen = fullscreen,
    }
}

pub(super) fn insights(app: &mut AppModel, action: InsightsAction) {
    match action {
        InsightsAction::ScrollLines(delta) => app.scroll_insights(delta),
        InsightsAction::ScrollPages(delta) => app.page_insights(delta),
        InsightsAction::ScrollToStart => app.nav.scroll.insights = 0,
        InsightsAction::ScrollToEnd => app.nav.scroll.insights = u16::MAX,
        InsightsAction::SetFullscreen(fullscreen) => app.nav.insights_fullscreen = fullscreen,
        InsightsAction::ToggleScope => {
            app.nav.insights_scope = app.nav.insights_scope.toggle();
            app.nav.scroll.reset_insights();
        }
        InsightsAction::CycleTimeframe => {
            app.nav.insights_timeframe = app.nav.insights_timeframe.next();
            app.nav.scroll.reset_insights();
        }
    }
}

pub(super) fn overlay(app: &mut AppModel, action: OverlayAction) -> AppResult<()> {
    match action {
        OverlayAction::ConfirmSelect(yes) => set_confirm_selection(app, yes),
        OverlayAction::Cancel => {
            if app.has_overlay() {
                if matches!(app.overlay, Overlay::NewJournal(_)) {
                    app.toast(ToastVariant::Info, "Cancelled");
                }
                app.close_overlay();
            }
        }
        OverlayAction::OpenHelp => app.open_help(),
        OverlayAction::HelpScroll(delta) => scroll_help(app, delta),
        OverlayAction::InputKey(key) => app.handle_text_input_key(key),
        OverlayAction::InputSelectAll => {
            if let Some(input) = app.focused_text_input_mut() {
                input.select_all();
            }
        }
        OverlayAction::ToggleHints => {
            app.state.ui.show_hints = !app.state.ui.show_hints;
            crate::config::save_state(&app.services.config_path, &app.state)?;
        }
        OverlayAction::ToggleJournals => {
            app.state.ui.show_journals = !app.state.ui.show_journals;
            if app.state.ui.show_journals {
                app.nav.focus = Focus::Journals;
            } else if app.nav.focus == Focus::Journals {
                app.nav.focus = Focus::Entries;
            }
            crate::config::save_state(&app.services.config_path, &app.state)?;
        }
    }
    Ok(())
}
