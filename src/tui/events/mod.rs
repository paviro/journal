mod actions;
mod keyboard;
mod mouse;
mod terminal;

#[cfg(test)]
use actions::view_selected;
pub(crate) use keyboard::handle_key;
#[cfg(test)]
use keyboard::{handle_enter, handle_right, move_focus_right, viewer_key_closes};
pub(crate) use mouse::handle_mouse;
#[cfg(test)]
use mouse::handle_mouse_in_area;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        crypto,
        tui::{
            app::{App, Focus},
            render,
        },
    };
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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
                    "---\ncreated_at: \"2026-07-01T10:{index:02}:00+02:00\"\n---\n\n# Entry {index}\nPreview {index}\n"
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

    #[test]
    fn enter_on_journals_moves_to_entries_like_right_arrow() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut enter_app = new_app(config.clone());
        let mut right_app = new_app(config);

        enter_app.focus = Focus::Journals;
        right_app.focus = Focus::Journals;

        handle_enter(&mut enter_app, true).unwrap();
        move_focus_right(&mut right_app, true);

        assert_eq!(enter_app.focus, Focus::Entries);
        assert_eq!(enter_app.focus, right_app.focus);
    }

    #[test]
    fn right_on_entry_opens_viewer_when_inline_entry_view_is_hidden() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "---\ntags: []\n---\n\n# A\nBody\n").unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        handle_right(&mut app, false).unwrap();

        assert!(app.viewer.is_some());
        assert_eq!(app.focus, Focus::Entries);
    }

    #[test]
    fn viewer_title_matches_entry_view_timestamp_title() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:23:00+02:00\"\n---\n\n# A\nBody\n",
        )
        .unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        handle_right(&mut app, false).unwrap();

        assert_eq!(app.viewer.as_ref().unwrap().title, "2026-07-01 10:23");
    }

    #[test]
    fn right_on_entry_focuses_entry_view_when_entry_view_is_available() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("a.md"), "---\ntags: []\n---\n\n# A\nBody\n").unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        handle_right(&mut app, true).unwrap();

        assert!(app.viewer.is_none());
        assert_eq!(app.focus, Focus::EntryView);
    }

    #[test]
    fn left_closes_viewer_only_when_entry_view_is_unavailable() {
        assert!(viewer_key_closes(KeyCode::Left, false));
        assert!(!viewer_key_closes(KeyCode::Left, true));
    }

    #[test]
    fn wide_journal_click_selects_journal_and_keeps_journal_focus() {
        let mut app = app_with_journals(&["alpha", "beta"]);
        app.focus = Focus::Journals;
        app.selected_entry_index = 3;
        app.entry_view_scroll = 10;
        let area = Rect::new(0, 0, 120, 20);
        let layout = render::tui_layout(area, &app);
        let journals = render::panel_inner(layout.journals.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                journals.x,
                journals.y + 1,
            ),
            area,
        )
        .unwrap();

        assert_eq!(app.selected_journal, 1);
        assert_eq!(app.selected_entry_index, 0);
        assert_eq!(app.entry_view_scroll, 0);
        assert_eq!(app.focus, Focus::Journals);
    }

    #[test]
    fn compact_journal_click_moves_to_entries() {
        let mut app = app_with_journals(&["work"]);
        app.focus = Focus::Journals;
        let area = Rect::new(0, 0, 57, 20);
        let layout = render::tui_layout(area, &app);
        let journals = render::panel_inner(layout.journals.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                journals.x,
                journals.y,
            ),
            area,
        )
        .unwrap();

        assert_eq!(app.selected_journal, 0);
        assert_eq!(app.focus, Focus::Entries);
    }

    #[test]
    fn journal_panel_click_without_row_focuses_journals_without_changing_selection() {
        let mut app = app_with_journals(&["alpha"]);
        app.focus = Focus::Entries;
        let area = Rect::new(0, 0, 120, 20);
        let layout = render::tui_layout(area, &app);
        let journals = render::panel_inner(layout.journals.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                journals.x,
                journals.y + 4,
            ),
            area,
        )
        .unwrap();

        assert_eq!(app.selected_journal, 0);
        assert_eq!(app.focus, Focus::Journals);
    }

    #[test]
    fn wheel_over_journals_scrolls_without_changing_selection() {
        let mut app = app_with_journals(&["a", "b", "c", "d", "e", "f", "g"]);
        app.focus = Focus::Entries;
        let area = Rect::new(0, 0, 120, 8);
        let layout = render::tui_layout(area, &app);
        let journals = render::panel_inner(layout.journals.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(MouseEventKind::ScrollDown, journals.x, journals.y),
            area,
        )
        .unwrap();

        assert_eq!(app.selected_journal, 0);
        assert_eq!(app.journal_scroll, 1);
        assert_eq!(app.focus, Focus::Entries);
    }

    #[test]
    fn wheel_over_entries_scrolls_without_changing_selection() {
        let mut app = app_with_entries(8);
        app.focus = Focus::Journals;
        let area = Rect::new(0, 0, 80, 8);
        let layout = render::tui_layout(area, &app);
        let entries = render::panel_inner(layout.entries.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(MouseEventKind::ScrollDown, entries.x, entries.y),
            area,
        )
        .unwrap();

        assert_eq!(app.selected_entry_index, 0);
        assert_eq!(app.entry_scroll, 1);
        assert_eq!(app.focus, Focus::Journals);
    }

    #[test]
    fn entry_click_selects_row_without_opening_viewer_when_entry_view_is_visible() {
        let mut app = app_with_entries(2);
        app.focus = Focus::Entries;
        let area = Rect::new(0, 0, 80, 12);
        let layout = render::tui_layout(area, &app);
        let entries = render::panel_inner(layout.entries.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                entries.x,
                entries.y + 2,
            ),
            area,
        )
        .unwrap();

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, 0);
        assert!(app.viewer.is_none());
    }

    #[test]
    fn entry_panel_click_without_entry_row_focuses_entries_without_opening_viewer() {
        let mut app = app_with_entries(1);
        app.focus = Focus::EntryView;
        let area = Rect::new(0, 0, 120, 12);
        let layout = render::tui_layout(area, &app);
        let entries = render::panel_inner(layout.entries.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                entries.x,
                entries.y,
            ),
            area,
        )
        .unwrap();

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, 0);
        assert!(app.viewer.is_none());
    }

    #[test]
    fn entry_panel_empty_space_click_focuses_entries_without_opening_viewer() {
        let mut app = app_with_entries(1);
        app.focus = Focus::EntryView;
        let area = Rect::new(0, 0, 120, 12);
        let layout = render::tui_layout(area, &app);
        let entries = render::panel_inner(layout.entries.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                entries.x,
                entries.y + 5,
            ),
            area,
        )
        .unwrap();

        assert_eq!(app.focus, Focus::Entries);
        assert_eq!(app.selected_entry_index, 0);
        assert!(app.viewer.is_none());
    }

    #[test]
    fn wheel_over_entry_view_scrolls_entry_view_only() {
        let mut app = app_with_entries(6);
        app.focus = Focus::Entries;
        let area = Rect::new(0, 0, 120, 20);
        let layout = render::tui_layout(area, &app);
        let entry_view = render::panel_inner(layout.entry_view.unwrap());

        handle_mouse_in_area(
            &mut app,
            mouse(MouseEventKind::ScrollDown, entry_view.x, entry_view.y),
            area,
        )
        .unwrap();

        assert_eq!(app.entry_view_scroll, 1);
        assert_eq!(app.entry_scroll, 0);
        assert_eq!(app.selected_entry_index, 0);
        assert_eq!(app.focus, Focus::EntryView);
    }

    #[test]
    fn viewer_wheel_scrolls_and_clicks_do_not_close() {
        let mut app = app_with_entries(1);
        view_selected(&mut app).unwrap();

        handle_mouse_in_area(
            &mut app,
            mouse(MouseEventKind::ScrollDown, 1, 1),
            Rect::new(0, 0, 80, 20),
        )
        .unwrap();
        assert_eq!(app.viewer.as_ref().unwrap().scroll, 1);

        handle_mouse_in_area(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
            Rect::new(0, 0, 80, 20),
        )
        .unwrap();
        assert!(app.viewer.is_some());
    }
}
