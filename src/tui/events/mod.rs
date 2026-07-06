mod action;
mod actions;
mod keyboard;
mod mouse;
mod terminal;

use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::{
    AppResult,
    tui::{
        app::{App, Focus, entry_view_is_available},
        render,
        state::Overlay,
    },
};

use action::Action;
use actions::{
    create_entry_in_selected_journal, delete_selected, delete_selected_journal, edit_selected,
    set_feelings_on_entry, set_metadata_on_entry, set_mood_on_entry, submit_new_journal,
    view_selected,
};
use keyboard::{keep_selection_visible, move_focus_left, move_focus_right};

pub(crate) use keyboard::handle_key;
pub(crate) use mouse::handle_mouse;

pub(crate) fn dispatch_action(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    action: Action,
) -> AppResult<bool> {
    use crate::tui::state::EditTagFocus;

    match action {
        Action::Quit => return Ok(true),

        Action::FocusLeft => move_focus_left(app),
        Action::FocusRight => {
            let available = entry_view_is_available(terminal.size()?.width);
            move_focus_right(app, available);
        }
        Action::MoveUp => {
            app.move_selection(-1);
            keep_selection_visible(terminal, app)?;
        }
        Action::MoveDown => {
            app.move_selection(1);
            keep_selection_visible(terminal, app)?;
        }

        Action::ScrollEntryView(delta) => app.scroll_entry_view(delta),
        Action::PageEntryView(delta) => app.page_entry_view(delta),
        Action::ScrollEntryViewToStart => app.scroll.entry_view = 0,
        Action::ScrollEntryViewToEnd => app.scroll.entry_view = u16::MAX,

        Action::BeginSearch => {
            app.begin_search();
        }
        Action::ExitSearch => {
            app.exit_search();
        }
        Action::EditSelected => {
            let snapshot = EntryViewSnapshot::capture(app);
            edit_selected(terminal, app)?;
            restore_entry_view_or_close(app, snapshot);
        }
        Action::ViewSelected => view_selected(app)?,
        Action::BeginDelete => app.begin_confirm_delete(),
        Action::ConfirmDelete => confirm_delete(app)?,
        Action::CancelOverlay => {
            if app.has_overlay() {
                if matches!(app.overlay, Overlay::NewJournal(_)) {
                    app.set_status("Cancelled");
                }
                app.close_overlay();
            }
        }
        Action::BeginEditTags => app.begin_edit_tags(),
        Action::BeginEditPeople => app.begin_edit_people(),
        Action::BeginEditActivities => app.begin_edit_activities(),
        Action::BeginEditFeelings => app.begin_edit_feelings(),
        Action::BeginEditMood => app.begin_edit_mood(),
        Action::NewEntry => {
            let snapshot = EntryViewSnapshot::capture(app);
            let restore_to_viewer = snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.focus == Focus::EntryView);
            let created = create_entry_in_selected_journal(terminal, app)?;
            if restore_to_viewer {
                let created_id = created.as_deref().and_then(journal_storage::entry_id);
                if let Some(id) = created_id {
                    if app.select_entry_by_id(&id, true) {
                        app.focus = Focus::EntryView;
                    } else {
                        restore_entry_view_or_close(app, snapshot);
                    }
                } else {
                    restore_entry_view_or_close(app, snapshot);
                }
            }
        }
        Action::NewJournal => app.begin_new_journal_input(),

        Action::JournalInputChar(ch) => {
            if let Some(input) = app.new_journal_input_mut() {
                input.push(ch);
            }
        }
        Action::JournalInputBackspace => {
            if let Some(input) = app.new_journal_input_mut() {
                input.pop();
            }
        }
        Action::JournalInputSubmit => submit_new_journal(app)?,

        Action::TagsMoveUp => {
            let list_height = tag_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_tag_state_mut() {
                state.move_up();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::TagsMoveDown => {
            let list_height = tag_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_tag_state_mut() {
                state.move_down();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::TagsToggle => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.toggle_selected();
            }
        }
        Action::TagsSwitchFocus => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.focus = match state.focus {
                    EditTagFocus::List => EditTagFocus::Input,
                    EditTagFocus::Input => EditTagFocus::List,
                };
            }
        }
        Action::TagsInput(ch) => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.input.push(ch);
                state.rebuild_filter();
            }
        }
        Action::TagsBackspace => {
            if let Some(state) = app.edit_tag_state_mut() {
                state.input.pop();
                state.rebuild_filter();
            }
        }
        Action::TagsAddFromInput => {
            if let Some(state) = app.edit_tag_state_mut() {
                let tag = state.input.trim().to_lowercase();
                if !tag.is_empty() && !state.selected.contains(&tag) {
                    state.selected.push(tag.clone());
                    if !state
                        .all_tags
                        .iter()
                        .any(|(t, _)| t.eq_ignore_ascii_case(&tag))
                    {
                        state.all_tags.push((tag, 0));
                    }
                }
                state.input.clear();
                state.rebuild_filter();
            }
        }
        Action::TagsSave => {
            let snapshot = EntryViewSnapshot::capture(app);
            let tags = app
                .edit_tag_state()
                .map(|s| s.selected.clone())
                .unwrap_or_default();
            let kind = app
                .edit_tag_state()
                .map(|s| s.kind)
                .unwrap_or(crate::tui::state::MetadataKind::Tags);
            set_metadata_on_entry(app, kind, &tags)?;
            restore_entry_view_or_close(app, snapshot);
            app.close_overlay();
        }

        Action::FeelingsMoveUp => {
            let list_height = feelings_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.move_up();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsMoveDown => {
            let list_height = feelings_dialog_list_height(terminal, app)?;
            if let Some(state) = app.edit_feeling_state_mut() {
                state.move_down();
                state.ensure_selected_visible(list_height);
            }
        }
        Action::FeelingsToggle => {
            if let Some(state) = app.edit_feeling_state_mut() {
                state.toggle_selected();
            }
        }
        Action::FeelingsSave => {
            let snapshot = EntryViewSnapshot::capture(app);
            let feelings = app
                .edit_feeling_state()
                .map(|s| s.selected.clone())
                .unwrap_or_default();
            set_feelings_on_entry(app, &feelings)?;
            restore_entry_view_or_close(app, snapshot);
            app.close_overlay();
        }

        Action::MoodDecrease => {
            if let Some(state) = app.edit_mood_state_mut()
                && state.draft > -5
            {
                state.draft -= 1;
            }
        }
        Action::MoodIncrease => {
            if let Some(state) = app.edit_mood_state_mut()
                && state.draft < 5
            {
                state.draft += 1;
            }
        }
        Action::MoodSave => {
            let snapshot = EntryViewSnapshot::capture(app);
            let mood = app.edit_mood_state().map(|s| s.draft);
            set_mood_on_entry(app, mood)?;
            restore_entry_view_or_close(app, snapshot);
            app.close_overlay();
        }
        Action::MoodClear => {
            let snapshot = EntryViewSnapshot::capture(app);
            let mood = app.edit_mood_state().and_then(|s| s.saved);
            if mood.is_some() {
                set_mood_on_entry(app, None)?;
            }
            restore_entry_view_or_close(app, snapshot);
            app.close_overlay();
        }

        Action::OpenImageViewer(index) => app.begin_image_viewer(index),
        Action::ImageViewerNext => app.image_viewer_step(1),
        Action::ImageViewerPrev => app.image_viewer_step(-1),

        Action::SearchInput(ch) => app.search_insert(ch),
        Action::SearchBackspace => app.search_backspace(),
        Action::SearchCursorLeft => app.search_cursor_left(),
        Action::SearchCursorRight => app.search_cursor_right(),

        Action::ToggleHints => {
            app.config.show_hints = !app.config.show_hints;
            crate::config::save_config(&app.config_path, &app.config)?;
        }

        Action::ToggleJournals => {
            app.config.show_journals = !app.config.show_journals;
            if app.config.show_journals {
                // Focus the column so narrow/medium layouts actually reveal it.
                app.focus = Focus::Journals;
            } else if app.focus == Focus::Journals {
                // Don't leave focus on a now-hidden pane.
                app.focus = Focus::Entries;
            }
            crate::config::save_config(&app.config_path, &app.config)?;
        }
    }

    Ok(false)
}

struct EntryViewSnapshot {
    id: String,
    focus: Focus,
    entry_view_scroll: u16,
}

impl EntryViewSnapshot {
    fn capture(app: &App) -> Option<Self> {
        let target = app.selected_entry_target()?;
        Some(Self {
            id: target.id,
            focus: app.focus,
            entry_view_scroll: app.scroll.entry_view,
        })
    }

    fn restore(self, app: &mut App) -> bool {
        if !app.select_entry_by_id(&self.id, false) {
            return false;
        }
        app.focus = self.focus;
        app.scroll.entry_view = self.entry_view_scroll;
        true
    }
}

fn restore_entry_view_or_close(app: &mut App, snapshot: Option<EntryViewSnapshot>) {
    let Some(snapshot) = snapshot else {
        return;
    };
    let was_in_viewer = snapshot.focus == Focus::EntryView;
    if !snapshot.restore(app) && was_in_viewer {
        app.focus = Focus::Entries;
        app.scroll.reset_entry_view();
    }
}

fn confirm_delete(app: &mut App) -> AppResult<()> {
    let is_journal = matches!(
        &app.overlay,
        Overlay::ConfirmDelete(crate::tui::state::DeleteContext::Journal { .. })
    );
    if is_journal {
        delete_selected_journal(app)?;
    } else {
        delete_selected(app)?;
    }
    app.close_overlay();
    app.focus = if is_journal {
        Focus::Journals
    } else {
        Focus::Entries
    };
    app.scroll.reset_entry_view();
    app.refresh()
}

fn terminal_area(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> AppResult<Rect> {
    let size = terminal.size()?;
    Ok(Rect::new(0, 0, size.width, size.height))
}

fn tag_dialog_list_height(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &App,
) -> AppResult<u16> {
    let filtered_len = app.edit_tag_state().map_or(0, |state| state.filtered.len());
    Ok(
        render::tags_dialog_layout(terminal_area(terminal)?, filtered_len)
            .list
            .height,
    )
}

fn feelings_dialog_list_height(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &App,
) -> AppResult<u16> {
    let all_len = app
        .edit_feeling_state()
        .map_or(0, |state| state.all_feelings.len());
    Ok(
        render::feelings_dialog_layout(terminal_area(terminal)?, all_len)
            .list
            .height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        tui::{
            app::{App, Focus, ScrollbarDrag},
            render, scroll,
            state::EditTagFocus,
        },
    };
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use journal_storage::JournalStore;
    use ratatui::layout::Rect;
    use std::fs;
    use tempfile::tempdir;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }

    fn down() -> MouseEventKind {
        MouseEventKind::Down(MouseButton::Left)
    }

    fn drag() -> MouseEventKind {
        MouseEventKind::Drag(MouseButton::Left)
    }

    fn up() -> MouseEventKind {
        MouseEventKind::Up(MouseButton::Left)
    }

    fn new_app(config: Config) -> App {
        let config_path = config.journal_root.join("config.toml");
        let store = JournalStore::for_config(&config_path, &config.journal_root).unwrap();
        App::new(config_path, config, store).unwrap()
    }

    fn app_with_journals(names: &[&str]) -> App {
        let dir = tempdir().unwrap();
        for name in names {
            fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        let config = Config::new(dir.path().to_path_buf(), "true");
        let app = new_app(config);
        std::mem::forget(dir);
        app
    }

    fn app_with_entries(count: usize) -> App {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        for index in 0..count {
            fs::write(
                entry_dir.join(format!("{index}.md")),
                format!(
                    "+++\ncreated_at = \"2026-07-01T10:{index:02}:00+02:00\"\n+++\n\n# Entry {index}\nPreview {index}\n"
                ),
            )
            .unwrap();
        }
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        std::mem::forget(dir);
        app
    }

    fn mouse_in_area(app: &mut App, event: MouseEvent, w: u16, h: u16) {
        mouse::handle_mouse_in_area(app, event, Rect::new(0, 0, w, h)).unwrap();
    }

    fn set_tag_dialog_items(app: &mut App, count: usize) {
        let state = app.edit_tag_state_mut().unwrap();
        state.all_tags = (0..count)
            .map(|index| (format!("tag-{index:02}"), index + 1))
            .collect();
        state.filtered = (0..count).collect();
        state.normalize_list_state();
    }

    #[test]
    fn enter_on_journals_moves_to_entries_like_right_arrow() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut enter_app = new_app(config.clone());
        let mut right_app = new_app(config);

        enter_app.focus = Focus::Journals;
        right_app.focus = Focus::Journals;

        // Enter and Right on Journals both resolve to move_focus_right
        move_focus_right(&mut enter_app, true);
        move_focus_right(&mut right_app, true);

        assert_eq!(enter_app.focus, Focus::Entries);
        assert_eq!(enter_app.focus, right_app.focus);
    }

    #[test]
    fn right_on_entry_expands_when_inline_entry_view_is_hidden() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\nBody\n").unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        // Right on Entries when not entry_view_available → ViewSelected → view_selected
        view_selected(&mut app).unwrap();

        assert_eq!(app.focus, Focus::EntryView);
    }

    #[test]
    fn expanded_entry_title_matches_entry_view_timestamp_title() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\ncreated_at = \"2026-07-01T10:23:00+02:00\"\n+++\n\n# A\nBody\n",
        )
        .unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        view_selected(&mut app).unwrap();

        let (title, _) = app.selected_entry_view().unwrap();
        assert_eq!(title, "Wednesday, 1 July 2026, 10:23");
    }

    #[test]
    fn right_on_entry_focuses_entry_view_when_entry_view_is_available() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\nBody\n").unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        // Right on Entries when entry_view_available → FocusRight → focus to EntryView
        move_focus_right(&mut app, true);

        assert_eq!(app.focus, Focus::EntryView);
    }

    #[test]
    fn typed_hint_ids_route_to_actions_without_string_parsing() {
        let mut app = app_with_entries(1);
        app.focus = Focus::Entries;

        assert_eq!(
            mouse::hint_id_to_action(&app, render::HintId::BeginEditTags),
            Some(Action::BeginEditTags)
        );
        assert_eq!(
            mouse::hint_id_to_action(&app, render::HintId::EditSelected),
            Some(Action::EditSelected)
        );
        assert_eq!(
            mouse::hint_id_to_action(&app, render::HintId::TagsToggle),
            None
        );

        app.begin_edit_tags();
        if let Some(state) = app.edit_tag_state_mut() {
            state.all_tags.push(("work".to_string(), 1));
            state.filtered.push(0);
        }
        assert_eq!(
            mouse::hint_id_to_action(&app, render::HintId::TagsToggle),
            Some(Action::TagsToggle)
        );
        assert_eq!(
            mouse::hint_id_to_action(&app, render::HintId::TagsSave),
            Some(Action::TagsSave)
        );
        assert_eq!(
            mouse::hint_id_to_action(&app, render::HintId::CancelOverlay),
            Some(Action::CancelOverlay)
        );
    }

    #[test]
    fn enter_in_metadata_input_saves_when_input_is_empty() {
        let mut app = app_with_entries(1);
        app.begin_edit_tags();
        let state = app.edit_tag_state_mut().unwrap();
        state.focus = EditTagFocus::Input;
        state.input.clear();

        assert_eq!(
            keyboard::key_to_action(
                &app,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
                true
            ),
            Some(Action::TagsSave)
        );

        app.edit_tag_state_mut().unwrap().input = "rust".to_string();
        assert_eq!(
            keyboard::key_to_action(
                &app,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
                true
            ),
            Some(Action::TagsAddFromInput)
        );
    }

    #[test]
    fn wide_journal_click_selects_journal_and_keeps_journal_focus() {
        let mut app = app_with_journals(&["alpha", "beta"]);
        app.focus = Focus::Journals;
        app.selected_entry_index = Some(3);
        app.scroll.entry_view = 10;
        let layout = render::tui_layout(Rect::new(0, 0, 120, 20), &app);
        let journals = layout.journals.unwrap().content;

        // Row 0 is the leading offset, rows 1-3 the first journal box, so the
        // second journal box starts at row 4.
        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                journals.x,
                journals.y + 4,
            ),
            120,
            20,
        );

        assert_eq!(app.selected_journal_index(), 1);
        assert_eq!(app.selected_entry_index, Some(0));
        assert_eq!(app.scroll.entry_view, 0);
        assert_eq!(app.focus, Focus::Journals);
    }

    #[test]
    fn compact_journal_click_moves_to_entries() {
        let mut app = app_with_journals(&["work"]);
        app.focus = Focus::Journals;
        let layout = render::tui_layout(Rect::new(0, 0, 57, 20), &app);
        let journals = layout.journals.unwrap().content;

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                journals.x,
                journals.y,
            ),
            57,
            20,
        );

        assert_eq!(app.selected_journal_index(), 0);
        assert_eq!(app.focus, Focus::Entries);
    }

    #[test]
    fn journal_panel_click_without_row_focuses_journals_without_changing_selection() {
        let mut app = app_with_journals(&["alpha"]);
        app.focus = Focus::Entries;
        let layout = render::tui_layout(Rect::new(0, 0, 130, 20), &app);
        let journals = layout.journals.unwrap().content;

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                journals.x,
                journals.y + 4,
            ),
            130,
            20,
        );

        assert_eq!(app.selected_journal_index(), 0);
        assert_eq!(app.focus, Focus::Journals);
    }

    #[test]
    fn wheel_over_journals_scrolls_without_changing_selection() {
        let mut app = app_with_journals(&["a", "b", "c", "d", "e", "f", "g"]);
        app.focus = Focus::Entries;
        let layout = render::tui_layout(Rect::new(0, 0, 130, 8), &app);
        let journals = layout.journals.unwrap().content;

        mouse_in_area(
            &mut app,
            mouse(MouseEventKind::ScrollDown, journals.x, journals.y),
            130,
            8,
        );

        assert_eq!(app.selected_journal_index(), 0);
        assert_eq!(app.journal_list.offset(), 1);
        assert_eq!(app.focus, Focus::Entries);
    }

    #[test]
    fn wheel_over_entries_scrolls_without_changing_selection() {
        let mut app = app_with_entries(8);
        app.focus = Focus::Journals;
        let layout = render::tui_layout(Rect::new(0, 0, 90, 8), &app);
        let entries = layout.entries.unwrap().panel.content;

        mouse_in_area(
            &mut app,
            mouse(MouseEventKind::ScrollDown, entries.x, entries.y),
            90,
            8,
        );

        assert_eq!(app.selected_entry_index, Some(0));
        assert_eq!(app.entry_list.offset(), 1);
        assert_eq!(app.focus, Focus::Journals);
    }

    #[test]
    fn entry_click_selects_row_without_opening_viewer_when_entry_view_is_visible() {
        let mut app = app_with_entries(2);
        app.focus = Focus::Journals;
        let layout = render::tui_layout(Rect::new(0, 0, 130, 12), &app);
        let geo = layout.entries.unwrap();
        let entries = geo.panel.content;
        let rows = render::entry_row_metadata(&app, geo.text_width);
        let y_off: u16 = rows
            .iter()
            .take_while(|row| row.entry_index != Some(1))
            .map(|row| row.height)
            .sum();

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                entries.x,
                entries.y + y_off,
            ),
            130,
            12,
        );

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, Some(1));
    }

    #[test]
    fn entry_panel_month_divider_click_deselects_to_journal_stats() {
        let mut app = app_with_entries(1);
        app.focus = Focus::EntryView;
        let layout = render::tui_layout(Rect::new(0, 0, 120, 12), &app);
        let entries = layout.entries.unwrap().panel.content;

        // The top row is the month divider, not an entry.
        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                entries.x,
                entries.y,
            ),
            120,
            12,
        );

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, None);
    }

    #[test]
    fn entry_panel_empty_space_click_deselects_to_journal_stats() {
        let mut app = app_with_entries(1);
        app.focus = Focus::EntryView;
        let layout = render::tui_layout(Rect::new(0, 0, 130, 20), &app);
        let geo = layout.entries.unwrap();
        let entries = geo.panel.content;
        let rows = render::entry_row_metadata(&app, geo.text_width);
        // First empty row below the (single entry's) list content.
        let total: u16 = rows.iter().map(|row| row.height).sum();

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                entries.x,
                entries.y + total,
            ),
            130,
            20,
        );

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, None);
    }

    #[test]
    fn wheel_over_entry_view_scrolls_entry_view_only() {
        let mut app = app_with_entries(6);
        app.focus = Focus::Entries;
        let layout = render::tui_layout(Rect::new(0, 0, 120, 20), &app);
        let entry_view = layout.entry_view.unwrap().content;

        mouse_in_area(
            &mut app,
            mouse(MouseEventKind::ScrollDown, entry_view.x, entry_view.y),
            120,
            20,
        );

        assert_eq!(app.scroll.entry_view, 1);
        assert_eq!(app.entry_list.offset(), 0);
        assert_eq!(app.selected_entry_index, Some(0));
        assert_eq!(app.focus, Focus::EntryView);
    }

    #[test]
    fn expanded_entry_wheel_scrolls_and_clicks_do_not_close() {
        let mut app = app_with_entries(1);
        view_selected(&mut app).unwrap();

        mouse_in_area(&mut app, mouse(MouseEventKind::ScrollDown, 1, 1), 80, 20);
        assert_eq!(app.scroll.entry_view, 1);

        mouse_in_area(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
            80,
            20,
        );
        assert_eq!(app.focus, Focus::EntryView);
    }

    #[test]
    fn metadata_refresh_restores_expanded_entry_view_and_scroll() {
        let mut app = app_with_entries(1);
        view_selected(&mut app).unwrap();
        app.scroll.entry_view = 7;

        let snapshot = EntryViewSnapshot::capture(&app);
        app.begin_edit_tags();
        super::actions::set_metadata_on_entry(
            &mut app,
            crate::tui::state::MetadataKind::Tags,
            &["work".to_string()],
        )
        .unwrap();
        restore_entry_view_or_close(&mut app, snapshot);
        app.close_overlay();

        assert_eq!(app.focus, Focus::EntryView);
        assert_eq!(app.scroll.entry_view, 7);
        assert_eq!(app.selected_entry_tags(), vec!["work".to_string()]);
        assert!(!app.has_overlay());
    }

    #[test]
    fn confirmed_delete_from_expanded_entry_closes_viewer() {
        let mut app = app_with_entries(1);
        view_selected(&mut app).unwrap();
        app.scroll.entry_view = 5;
        app.begin_confirm_delete();

        assert_eq!(app.focus, Focus::EntryView);

        confirm_delete(&mut app).unwrap();

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.scroll.entry_view, 0);
        assert_eq!(app.current_entry_list_len(), 0);
        assert!(!app.has_overlay());
    }

    #[test]
    fn search_from_entry_view_resets_focus_and_scroll() {
        let mut app = app_with_entries(1);
        view_selected(&mut app).unwrap();
        app.scroll.entry_view = 5;

        app.begin_search();

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.mode, crate::tui::app::Mode::Search);
        assert_eq!(app.scroll.entry_view, 0);
    }

    #[test]
    fn select_created_entry_path_opens_expanded_entry_view() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let entry_dir = root.join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "+++\ntags = []\n+++\n\n# Existing\nBody\n",
        )
        .unwrap();

        let config = Config::new(root.clone(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        view_selected(&mut app).unwrap();
        app.scroll.entry_view = 9;

        let store = JournalStore::for_config(&root.join("config.toml"), &root).unwrap();
        let created = store
            .create_entry_with_body(
                "work",
                "# Created\nBody\n",
                &journal_storage::Metadata::default(),
            )
            .unwrap();
        app.refresh().unwrap();
        let created_id = journal_storage::entry_id(&created).unwrap();
        assert!(app.select_entry_by_id(&created_id, true));
        app.focus = Focus::EntryView;

        assert_eq!(app.focus, Focus::EntryView);
        assert_eq!(app.scroll.entry_view, 0);
        assert_eq!(app.selected_entry_target().unwrap().path, created);
    }

    #[test]
    fn wheel_over_tag_dialog_list_scrolls_without_selection_or_toggle_change() {
        let mut app = app_with_entries(1);
        app.begin_edit_tags();
        set_tag_dialog_items(&mut app, 20);
        let layout = render::tags_dialog_layout(Rect::new(0, 0, 120, 20), 20);

        mouse_in_area(
            &mut app,
            mouse(MouseEventKind::ScrollDown, layout.list.x, layout.list.y),
            120,
            20,
        );

        let state = app.edit_tag_state().unwrap();
        assert_eq!(state.offset(), 1);
        assert_eq!(state.selected_index(), Some(0));
        assert!(state.selected.is_empty());
    }

    #[test]
    fn click_on_tag_dialog_row_selects_and_toggles_it() {
        let mut app = app_with_entries(1);
        app.begin_edit_tags();
        set_tag_dialog_items(&mut app, 5);
        let layout = render::tags_dialog_layout(Rect::new(0, 0, 120, 20), 5);

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                layout.list.x,
                layout.list.y + 2,
            ),
            120,
            20,
        );

        let state = app.edit_tag_state().unwrap();
        assert_eq!(state.selected_index(), Some(2));
        assert_eq!(state.selected, vec!["tag-02"]);
    }

    #[test]
    fn click_on_tag_dialog_placeholder_row_does_not_toggle() {
        let mut app = app_with_entries(1);
        app.begin_edit_tags();
        let state = app.edit_tag_state_mut().unwrap();
        state.all_tags = vec![("work".to_string(), 1)];
        state.filtered.clear();
        state.input = "missing".to_string();
        state.normalize_list_state();
        let layout = render::tags_dialog_layout(Rect::new(0, 0, 120, 12), 0);

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                layout.list.x,
                layout.list.y,
            ),
            120,
            12,
        );

        let state = app.edit_tag_state().unwrap();
        assert_eq!(state.selected_index(), None);
        assert!(state.selected.is_empty());
    }

    #[test]
    fn click_on_tag_input_row_switches_focus_to_input() {
        let mut app = app_with_entries(1);
        app.begin_edit_tags();
        set_tag_dialog_items(&mut app, 3);
        let layout = render::tags_dialog_layout(Rect::new(0, 0, 120, 16), 3);

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                layout.input.x,
                layout.input.y,
            ),
            120,
            16,
        );

        assert_eq!(app.edit_tag_state().unwrap().focus, EditTagFocus::Input);
    }

    #[test]
    fn click_on_feeling_dialog_row_selects_and_toggles_it() {
        let mut app = app_with_entries(1);
        app.begin_edit_feelings();
        let all_len = app.edit_feeling_state().unwrap().all_feelings.len();
        let layout = render::feelings_dialog_layout(Rect::new(0, 0, 120, 20), all_len);

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                layout.list.x,
                layout.list.y + 1,
            ),
            120,
            20,
        );

        let state = app.edit_feeling_state().unwrap();
        assert_eq!(state.selected_index(), Some(1));
        assert_eq!(state.selected, vec![state.all_feelings[1].clone()]);
    }

    #[test]
    fn click_and_drag_on_mood_bar_set_nearest_scores() {
        let mut app = app_with_entries(1);
        app.begin_edit_mood();
        let layout = render::mood_dialog_layout(Rect::new(0, 0, 120, 20));

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                layout.bar.x,
                layout.bar.y,
            ),
            120,
            20,
        );
        assert_eq!(app.edit_mood_state().unwrap().draft, -5);

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                layout.bar.x + layout.bar.width / 2,
                layout.bar.y,
            ),
            120,
            20,
        );
        assert_eq!(app.edit_mood_state().unwrap().draft, 0);

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Drag(MouseButton::Left),
                layout.bar.x + layout.bar.width - 1,
                layout.bar.y,
            ),
            120,
            20,
        );
        assert_eq!(app.edit_mood_state().unwrap().draft, 5);
    }

    /// The entry-list scrollbar geometry for a 60-entry list in a 120×20 area.
    struct EntryBarFixture {
        app: App,
        area: Rect,
        bar: Rect,
        total: usize,
        viewport: u16,
        max: usize,
    }

    fn entry_bar_fixture() -> EntryBarFixture {
        let app = app_with_entries(60);
        let entries = render::tui_layout(Rect::new(0, 0, 120, 20), &app)
            .entries
            .expect("entries panel");
        let area = entries.panel.area;
        let cache = app.entry_rows(entries.text_width);
        let total = cache.total_height;
        let viewport = entries.viewport_height;
        let max = total.saturating_sub(viewport as usize);
        assert!(max > 0, "entry list should overflow so a bar is drawn");
        EntryBarFixture {
            bar: scroll::scrollbar_bar_rect(area),
            app,
            area,
            total,
            viewport,
            max,
        }
    }

    #[test]
    fn scrollbar_arrows_step_one_line_without_dragging() {
        let EntryBarFixture {
            mut app, bar, max, ..
        } = entry_bar_fixture();
        let up_arrow = bar.y;
        let down_arrow = bar.y + bar.height - 1;

        // The down arrow steps one line down; no drag begins.
        mouse_in_area(&mut app, mouse(down(), bar.x, down_arrow), 120, 20);
        assert_eq!(app.entry_list.offset(), 1);
        assert!(app.scrollbar.active.is_none());
        assert_eq!(app.focus, Focus::Entries);

        // The up arrow steps back.
        mouse_in_area(&mut app, mouse(down(), bar.x, up_arrow), 120, 20);
        assert_eq!(app.entry_list.offset(), 0);
        assert!(app.scrollbar.active.is_none());
        assert!(max > 1);
    }

    #[test]
    fn scrollbar_thumb_press_grabs_without_jumping() {
        let EntryBarFixture {
            mut app,
            bar,
            total,
            viewport,
            max,
            ..
        } = entry_bar_fixture();
        *app.entry_list.offset_mut() = max / 2;
        let before = app.entry_list.offset();

        let position = scroll::scrollbar_position(before, total, viewport);
        let (thumb_top, thumb_len) =
            scroll::scrollbar_thumb(bar, total, viewport, position).expect("thumb");

        // Pressing straight on the thumb grabs it and leaves the scroll untouched.
        mouse_in_area(
            &mut app,
            mouse(down(), bar.x, thumb_top + thumb_len / 2),
            120,
            20,
        );
        assert_eq!(app.entry_list.offset(), before);
        assert_eq!(app.scrollbar.active, Some(ScrollbarDrag::EntryList));
    }

    #[test]
    fn scrollbar_track_press_jumps_then_drag_tracks_the_cursor() {
        let EntryBarFixture {
            mut app,
            area,
            bar,
            max,
            ..
        } = entry_bar_fixture();
        let bottom_track = bar.y + bar.height - 2; // last track row, above the down arrow
        let top_track = bar.y + 1; // first track row, below the up arrow

        // Press empty track near the bottom → thumb jumps down under the cursor.
        mouse_in_area(&mut app, mouse(down(), bar.x, bottom_track), 120, 20);
        assert_eq!(app.scrollbar.active, Some(ScrollbarDrag::EntryList));
        assert!(
            app.entry_list.offset() > max / 2,
            "expected a large jump, got {}",
            app.entry_list.offset()
        );

        // Drag to the top, cursor drifted off the bar column → scroll to 0.
        mouse_in_area(&mut app, mouse(drag(), 0, top_track), 120, 20);
        assert_eq!(app.entry_list.offset(), 0);

        // Release clears the drag.
        mouse_in_area(&mut app, mouse(up(), 0, top_track), 120, 20);
        assert!(app.scrollbar.active.is_none());

        // The grab region spans the bar column plus one on each side.
        for col in [bar.x - 1, bar.x + 1] {
            assert!(col >= area.x && col < area.x + area.width + 1);
            mouse_in_area(&mut app, mouse(down(), col, bottom_track), 120, 20);
            assert_eq!(app.scrollbar.active, Some(ScrollbarDrag::EntryList));
            mouse_in_area(&mut app, mouse(up(), col, bottom_track), 120, 20);
        }
    }

    #[test]
    fn scrollbar_track_press_scrolls_journals() {
        let names: Vec<String> = (0..60).map(|i| format!("journal-{i:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut app = app_with_journals(&refs);
        let journals = render::tui_layout(Rect::new(0, 0, 120, 20), &app)
            .journals
            .expect("journals panel");
        let bar = scroll::scrollbar_bar_rect(journals.area);
        let per_page =
            render::journals_per_page(render::journal_list_rect(journals.content).height);
        let max = app.journals.len().saturating_sub(per_page as usize);
        assert!(max > 0, "journals list should overflow so a bar is drawn");

        // Press the bottom track row → thumb jumps down.
        mouse_in_area(
            &mut app,
            mouse(down(), bar.x, bar.y + bar.height - 2),
            120,
            20,
        );
        assert_eq!(app.scrollbar.active, Some(ScrollbarDrag::Journals));
        assert!(app.journal_list.offset() > 0);
    }
}
