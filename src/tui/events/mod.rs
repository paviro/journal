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
    create_entry_in_selected_journal, delete_selected, edit_selected, set_feelings_on_entry,
    set_mood_on_entry, set_tags_on_entry, submit_new_journal, view_selected,
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
            close_expanded_entry_view(app);
            app.begin_search();
        }
        Action::ExitSearch => {
            close_expanded_entry_view(app);
            app.exit_search();
        }
        Action::EditSelected => edit_selected(terminal, app)?,
        Action::ViewSelected => view_selected(app)?,
        Action::BeginDelete => {
            close_expanded_entry_view(app);
            app.begin_confirm_delete();
        }
        Action::ConfirmDelete => {
            delete_selected(app)?;
            app.close_overlay();
            app.refresh()?;
        }
        Action::CancelOverlay => {
            if app.entry_view_expanded {
                app.entry_view_expanded = false;
                app.focus = Focus::Entries;
            } else {
                if matches!(app.overlay, Overlay::NewJournal(_)) {
                    app.set_status("Cancelled");
                }
                app.close_overlay();
            }
        }
        Action::BeginEditTags => {
            close_expanded_entry_view(app);
            app.begin_edit_tags();
        }
        Action::BeginEditFeelings => {
            close_expanded_entry_view(app);
            app.begin_edit_feelings();
        }
        Action::BeginEditMood => {
            close_expanded_entry_view(app);
            app.begin_edit_mood();
        }
        Action::NewEntry => {
            close_expanded_entry_view(app);
            create_entry_in_selected_journal(terminal, app)?;
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
            let tags = app
                .edit_tag_state()
                .map(|s| s.selected.clone())
                .unwrap_or_default();
            set_tags_on_entry(app, &tags)?;
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
            let feelings = app
                .edit_feeling_state()
                .map(|s| s.selected.clone())
                .unwrap_or_default();
            set_feelings_on_entry(app, &feelings)?;
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
            let mood = app.edit_mood_state().map(|s| s.draft);
            set_mood_on_entry(app, mood)?;
            app.close_overlay();
        }
        Action::MoodClear => {
            let mood = app.edit_mood_state().and_then(|s| s.saved);
            if mood.is_some() {
                set_mood_on_entry(app, None)?;
            }
            app.close_overlay();
        }

        Action::SearchInput(ch) => {
            app.search.query.push(ch);
            app.update_search_results();
        }
        Action::SearchBackspace => {
            app.search.query.pop();
            app.update_search_results();
        }
    }

    Ok(false)
}

fn close_expanded_entry_view(app: &mut App) {
    if app.entry_view_expanded {
        app.entry_view_expanded = false;
        app.focus = Focus::EntryView;
    }
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
        crypto,
        tui::{
            app::{App, Focus},
            render,
            state::EditTagFocus,
        },
    };
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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

    fn new_app(config: Config) -> App {
        let encryption_paths = crypto::EncryptionPaths::for_config(
            &config.journal_root.join("config.toml"),
            &config.journal_root,
        )
        .unwrap();
        App::new(config, encryption_paths).unwrap()
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

        assert!(app.entry_view_expanded);
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

        assert!(!app.entry_view_expanded);
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
    fn wide_journal_click_selects_journal_and_keeps_journal_focus() {
        let mut app = app_with_journals(&["alpha", "beta"]);
        app.focus = Focus::Journals;
        app.selected_entry_index = 3;
        app.scroll.entry_view = 10;
        let layout = render::tui_layout(Rect::new(0, 0, 120, 20), &app);
        let journals = layout.journals.unwrap().content;

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                journals.x,
                journals.y + 1,
            ),
            120,
            20,
        );

        assert_eq!(app.selected_journal_index(), 1);
        assert_eq!(app.selected_entry_index, 0);
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

        assert_eq!(app.selected_entry_index, 0);
        assert_eq!(app.entry_list.offset(), 1);
        assert_eq!(app.focus, Focus::Journals);
    }

    #[test]
    fn entry_click_selects_row_without_opening_viewer_when_entry_view_is_visible() {
        let mut app = app_with_entries(2);
        app.focus = Focus::Entries;
        let layout = render::tui_layout(Rect::new(0, 0, 80, 12), &app);
        let entries = layout.entries.unwrap().panel.content;

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                entries.x,
                entries.y + 2,
            ),
            80,
            12,
        );

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, 0);
        assert!(!app.entry_view_expanded);
    }

    #[test]
    fn entry_panel_click_without_entry_row_focuses_entries_without_opening_viewer() {
        let mut app = app_with_entries(1);
        app.focus = Focus::EntryView;
        let layout = render::tui_layout(Rect::new(0, 0, 120, 12), &app);
        let entries = layout.entries.unwrap().panel.content;

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
        assert_eq!(app.selected_entry_index, 0);
        assert!(!app.entry_view_expanded);
    }

    #[test]
    fn entry_panel_empty_space_click_focuses_entries_without_opening_viewer() {
        let mut app = app_with_entries(1);
        app.focus = Focus::EntryView;
        let layout = render::tui_layout(Rect::new(0, 0, 130, 12), &app);
        let entries = layout.entries.unwrap().panel.content;

        mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                entries.x,
                entries.y + 5,
            ),
            130,
            12,
        );

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, 0);
        assert!(!app.entry_view_expanded);
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
        assert_eq!(app.selected_entry_index, 0);
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
        assert!(app.entry_view_expanded);
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
}
