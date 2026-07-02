mod chrome;
mod dialogs;
mod entries;
mod journals;
mod layout;
mod markdown_panel;
mod stats;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    widgets::Paragraph,
};

use super::app::{App, entry_view_is_available};
#[cfg(test)]
pub(crate) use super::entry_rows::{
    EntryRowMeta, entry_day_label, entry_list_lines, entry_month_label,
};
pub(crate) use super::entry_rows::{
    ensure_entry_visible, entry_row_metadata, total_entry_row_height,
};
pub(crate) use super::hit_test::{
    entry_index_at, feeling_at_point, journal_index_at, panel_inner, point_in_rect, tag_at_point,
};
pub(crate) use super::scroll::{
    clamp_scroll, ensure_index_visible, scroll_offset, scrollbar_position, viewer_scroll,
};
#[cfg(test)]
pub(crate) use chrome::panel_title;
pub(crate) use chrome::{
    centered_rect, footer_text, panel_block, panel_content_inner, render_vertical_scrollbar,
    selected_style,
};
use dialogs::{
    draw_confirm_delete, draw_edit_feelings_dialog, draw_edit_mood_dialog, draw_edit_tags_dialog,
    draw_new_journal_input,
};
use entries::draw_entry_list;
use journals::draw_journals;
pub(crate) use layout::{TuiLayout, tui_layout};
use markdown_panel::draw_selected_entry_view;
#[cfg(test)]
pub(crate) use markdown_panel::markdown_theme;
use stats::draw_journal_stats;
#[cfg(test)]
pub(crate) use stats::{centered_stats_layout, journal_stats};

pub(crate) fn draw(frame: &mut Frame<'_>, app: &mut App) {
    if app.entry_view_expanded {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        draw_selected_entry_view(frame, chunks[0], app);
        frame.render_widget(
            Paragraph::new(" close (enter/esc) | edit (e) | quit (q)"),
            chunks[1],
        );
        return;
    }

    app.normalize_focus(entry_view_is_available(frame.area().width));
    let layout = tui_layout(frame.area(), app);

    if let Some(area) = layout.journals {
        draw_journals(frame, area, app);
    }
    if let Some(area) = layout.entries {
        draw_entry_list(frame, area, app);
    }
    if let Some(area) = layout.stats {
        draw_journal_stats(frame, area, app);
    } else if let Some(area) = layout.entry_view {
        draw_selected_entry_view(frame, area, app);
    }

    let footer_text = footer_text(app, layout.entry_view_visible);
    let footer = Paragraph::new(footer_text);
    frame.render_widget(footer, layout.footer);

    if app.is_confirming_delete() {
        draw_confirm_delete(frame);
    }

    if let Some(input) = app.new_journal_input() {
        draw_new_journal_input(frame, input);
    }

    if let Some(state) = app.edit_tag_state_mut() {
        draw_edit_tags_dialog(frame, state);
    }

    if let Some(state) = app.edit_feeling_state_mut() {
        draw_edit_feelings_dialog(frame, state);
    }

    if let Some(state) = app.edit_mood_state() {
        draw_edit_mood_dialog(frame, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        crypto,
        storage::{Entry, EntryEncryptionState},
        tui::{
            app::{Focus, INLINE_ENTRY_VIEW_MIN_WIDTH, Mode},
            state::{EditTagFocus, EditTagState},
        },
    };
    use ratatui::{
        Terminal,
        backend::TestBackend,
        layout::Rect,
        style::{Color, Modifier},
    };
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn new_app(config: Config) -> App {
        let encryption_paths = crypto::EncryptionPaths::for_config(
            &config.journal_root.join("config.toml"),
            &config.journal_root,
        )
        .unwrap();
        App::new(config, encryption_paths).unwrap()
    }

    fn app_with_entry() -> App {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let entry_dir = root.join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n...\n\n# A\nBody\n",
        )
        .unwrap();

        let config = Config::new(root, "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app
    }

    fn render_text(mut app: App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn render_app(mut app: App, width: u16, height: u16) -> TestBackend {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        terminal.backend().clone()
    }

    fn render_edit_tags_dialog_text(mut state: EditTagState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| dialogs::draw_edit_tags_dialog(frame, &mut state))
            .unwrap();

        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn layout_places_hit_targets_in_three_columns() {
        let mut app = app_with_entry();
        app.focus = Focus::Entries;

        let layout = tui_layout(Rect::new(0, 0, 120, 20), &app);

        assert!(!layout.single_panel);
        assert!(layout.entry_view_visible);
        assert_eq!(layout.journals.unwrap(), Rect::new(0, 0, 18, 19));
        assert_eq!(layout.entries.unwrap(), Rect::new(18, 0, 42, 19));
        assert_eq!(layout.entry_view.unwrap(), Rect::new(60, 0, 60, 19));
        assert_eq!(layout.footer, Rect::new(0, 19, 120, 1));
    }

    #[test]
    fn layout_keeps_three_columns_at_minimum_inline_width() {
        let mut app = app_with_entry();
        app.focus = Focus::Entries;

        let layout = tui_layout(Rect::new(0, 0, INLINE_ENTRY_VIEW_MIN_WIDTH, 20), &app);

        assert!(!layout.single_panel);
        assert!(layout.entry_view_visible);
        assert_eq!(layout.journals.unwrap(), Rect::new(0, 0, 18, 19));
        assert_eq!(layout.entries.unwrap(), Rect::new(18, 0, 42, 19));
        assert_eq!(layout.entry_view.unwrap(), Rect::new(60, 0, 40, 19));
    }

    #[test]
    fn layout_places_hit_targets_in_two_columns_without_inline_entry_view() {
        let mut app = app_with_entry();
        app.focus = Focus::Journals;

        let layout = tui_layout(Rect::new(0, 0, 80, 20), &app);

        assert!(!layout.single_panel);
        assert!(!layout.entry_view_visible);
        assert_eq!(layout.journals.unwrap(), Rect::new(0, 0, 18, 19));
        assert_eq!(layout.entries.unwrap(), Rect::new(18, 0, 62, 19));
        assert!(layout.entry_view.is_none());
    }

    #[test]
    fn layout_shifts_two_columns_to_entries_and_preview_when_entries_are_active() {
        let mut app = app_with_entry();
        app.focus = Focus::Entries;

        let layout = tui_layout(Rect::new(0, 0, 80, 20), &app);

        assert!(!layout.single_panel);
        assert!(layout.entry_view_visible);
        assert!(layout.journals.is_none());
        assert_eq!(layout.entries.unwrap(), Rect::new(0, 0, 42, 19));
        assert_eq!(layout.entry_view.unwrap(), Rect::new(42, 0, 38, 19));
    }

    #[test]
    fn layout_uses_single_compact_panel_for_active_focus() {
        let mut app = app_with_entry();
        app.focus = Focus::Journals;

        let journals = tui_layout(Rect::new(0, 0, 57, 20), &app);
        assert!(journals.single_panel);
        assert_eq!(journals.journals.unwrap(), Rect::new(0, 0, 57, 19));
        assert!(journals.entries.is_none());

        app.focus = Focus::Entries;
        let entries = tui_layout(Rect::new(0, 0, 57, 20), &app);
        assert!(entries.single_panel);
        assert_eq!(entries.entries.unwrap(), Rect::new(0, 0, 57, 19));
        assert!(entries.journals.is_none());
    }

    #[test]
    fn viewer_scroll_clamps_to_rendered_content_height() {
        assert_eq!(viewer_scroll(100, 20, 8), 12);
        assert_eq!(viewer_scroll(5, 4, 8), 0);
    }

    #[test]
    fn viewer_scroll_saturates_large_rendered_content_height() {
        assert_eq!(viewer_scroll(u16::MAX, 100_000, 8), u16::MAX);
    }

    #[test]
    fn scrollbar_position_reaches_end_at_viewer_bottom() {
        let line_count = 40;
        let height = 20;
        let scroll = viewer_scroll(u16::MAX, line_count, height);

        assert_eq!(scroll, 20);
        assert_eq!(scrollbar_position(scroll, line_count, height), 39);
    }

    #[test]
    fn scrollbar_position_stays_at_start_when_content_fits() {
        assert_eq!(scrollbar_position(0, 4, 8), 0);
    }

    #[test]
    fn edit_tags_dialog_keeps_help_visible_below_spacer() {
        let all_tags: Vec<(String, usize)> = (0..20)
            .map(|index| (format!("tag-{index:02}"), index))
            .collect();
        let filtered: Vec<usize> = (0..all_tags.len()).collect();
        let rendered = render_edit_tags_dialog_text(
            EditTagState {
                all_tags,
                filtered,
                selected: Vec::new(),
                cursor: 0,
                scroll: 0,
                input: String::new(),
                focus: EditTagFocus::List,
            },
            200,
            20,
        );

        assert!(rendered.contains(">[ ] tag-00 (0)"));
        assert!(rendered.contains(" toggle (space) | input (tab) | save (enter) | cancel (esc)"));
    }

    #[test]
    fn edit_tags_dialog_counts_no_matches_row_when_sizing() {
        let rendered = render_edit_tags_dialog_text(
            EditTagState {
                all_tags: vec![("work".to_string(), 1)],
                filtered: Vec::new(),
                selected: Vec::new(),
                cursor: 0,
                scroll: 0,
                input: "missing".to_string(),
                focus: EditTagFocus::Input,
            },
            200,
            12,
        );

        assert!(rendered.contains(" (no matches)"));
        assert!(rendered.contains(" add (enter) | list (tab) | cancel (esc)"));
    }

    #[test]
    fn entry_hit_testing_ignores_headers_and_maps_three_line_entries() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n...\n\n# A\nFirst preview\n",
        )
        .unwrap();
        fs::write(
            entry_dir.join("b.md"),
            "---\ncreated_at: \"2026-07-01T11:00:00+02:00\"\n...\n\n# B\nSecond preview\n",
        )
        .unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        let area = Rect::new(0, 0, 40, 10);
        // text_width=10 gives entries height 3 (title + 2 preview lines)
        let rows = entry_row_metadata(&app, 10);

        assert_eq!(
            rows,
            vec![
                EntryRowMeta {
                    entry_index: None,
                    height: 3,
                },
                EntryRowMeta {
                    entry_index: None,
                    height: 1,
                },
                EntryRowMeta {
                    entry_index: Some(0),
                    height: 3,
                },
                EntryRowMeta {
                    entry_index: Some(1),
                    height: 3,
                },
            ]
        );
        assert_eq!(entry_index_at(area, 1, 1, 0, &rows), None);
        assert_eq!(entry_index_at(area, 1, 2, 0, &rows), None);
        assert_eq!(entry_index_at(area, 1, 3, 0, &rows), None);
        assert_eq!(entry_index_at(area, 1, 4, 0, &rows), None);
        assert_eq!(entry_index_at(area, 1, 5, 0, &rows), Some(0));
        assert_eq!(entry_index_at(area, 1, 6, 0, &rows), Some(0));
        assert_eq!(entry_index_at(area, 1, 7, 0, &rows), Some(0));
        assert_eq!(entry_index_at(area, 1, 8, 0, &rows), Some(1));
        assert_eq!(entry_index_at(area, 1, 1, 2, &rows), None);
    }

    #[test]
    fn markdown_theme_uses_terminal_default_foregrounds() {
        let theme = markdown_theme();

        assert_eq!(theme.text_color, Color::Reset);
        assert_eq!(theme.muted_text_color, Color::Reset);
        assert_eq!(theme.primary_color, Color::Reset);
        assert_eq!(theme.secondary_color, Color::Reset);
        assert_eq!(theme.accent_yellow, Color::Reset);
        assert_eq!(theme.code_colors.variable, Color::Reset);
    }

    #[test]
    fn entry_view_renders_feelings_metadata() {
        let dir = tempdir().unwrap();
        let entry_dir = dir.path().join("work").join("2026-07-01");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\nfeelings:\n  - calm\n  - focused\n...\n\n# A\nBody\n",
        )
        .unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::EntryView;

        let rendered = render_text(app, 120, 20);

        assert!(rendered.contains("Feelings: calm | focused"));
    }

    #[test]
    fn focused_panel_titles_have_ascii_focus_marker() {
        assert_eq!(panel_title("Entries", true), " >> Entries ");
        assert_eq!(panel_title("Entries", false), " Entries ");
    }

    #[test]
    fn compact_render_shows_only_the_active_step() {
        let mut journals_app = app_with_entry();
        journals_app.focus = Focus::Journals;
        let journals = render_text(journals_app, 57, 16);
        assert!(journals.contains(">> Journals"));
        assert!(!journals.contains(" Entries "));
        assert!(!journals.contains("2026-07-01 10:00"));

        let mut entries_app = app_with_entry();
        entries_app.focus = Focus::Entries;
        let entries = render_text(entries_app, 57, 16);
        assert!(entries.contains(">> Entries"));
        assert!(!entries.contains(" Journals "));
        assert!(!entries.contains("2026-07-01 10:00"));

        let mut entry_view_focus_app = app_with_entry();
        entry_view_focus_app.focus = Focus::EntryView;
        let entry_view_focus = render_text(entry_view_focus_app, 57, 16);
        assert!(entry_view_focus.contains(">> Entries"));
        assert!(!entry_view_focus.contains(" Journals "));
        assert!(!entry_view_focus.contains("2026-07-01 10:00"));
    }

    #[test]
    fn two_column_render_follows_active_column_pair() {
        let mut journals_app = app_with_entry();
        journals_app.focus = Focus::Journals;
        let journals = render_text(journals_app, 80, 16);
        assert!(journals.contains(">> Journals"));
        assert!(journals.contains(" Entries "));
        assert!(!journals.contains("2026-07-01 10:00"));

        let mut entries_app = app_with_entry();
        entries_app.focus = Focus::Entries;
        let entries = render_text(entries_app, 80, 16);
        assert!(entries.contains(">> Entries"));
        assert!(!entries.contains(" Journals "));
        assert!(entries.contains("2026-07-01 10:00"));
    }

    #[test]
    fn selected_journal_and_entry_remain_reversed_when_entry_view_is_focused() {
        let mut app = app_with_entry();
        app.focus = Focus::EntryView;

        let backend = render_app(app, 120, 20);
        let buffer = backend.buffer();

        assert!(
            buffer
                .cell((2, 1))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            buffer
                .cell((20, 6))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn selected_entry_is_not_reversed_when_journals_are_focused() {
        let mut app = app_with_entry();
        app.focus = Focus::Journals;

        let backend = render_app(app, 120, 20);
        let buffer = backend.buffer();

        assert!(
            buffer
                .cell((2, 1))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !buffer
                .cell((19, 3))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn journal_stats_summarizes_selected_journal() {
        let app = app_with_entry();

        let stats = journal_stats(&app).unwrap();

        assert_eq!(stats.name, "work");
        assert_eq!(stats.entry_count, 1);
        assert_eq!(stats.active_days, 1);
        assert_eq!(stats.year_range, "2026");
    }

    #[test]
    fn journal_stats_handles_empty_journals() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");

        let stats = journal_stats(&app).unwrap();

        assert_eq!(stats.name, "work");
        assert_eq!(stats.entry_count, 0);
        assert_eq!(stats.active_days, 0);
        assert_eq!(stats.year_range, "No dated entries");
    }

    #[test]
    fn centered_stats_layout_places_identity_above_metric_cards() {
        let layout = centered_stats_layout(Rect {
            x: 10,
            y: 3,
            width: 80,
            height: 24,
        });

        assert_eq!(layout.identity.y, 8);
        assert_eq!(layout.identity.height, 6);
        assert_eq!(layout.entries.y, 14);
        assert_eq!(layout.days.y, 14);
        assert!(layout.entries.x < layout.days.x);
        assert_eq!(layout.entries.height, 6);
        assert_eq!(layout.days.height, 6);
    }

    #[test]
    fn journal_footer_omits_entry_actions() {
        let mut app = app_with_entry();
        app.focus = Focus::Journals;

        let text = footer_text(&app, true);

        assert!(!text.contains("view (enter)"));
        assert!(!text.contains("edit (e)"));
        assert!(!text.contains("delete (d)"));
    }

    #[test]
    fn entries_footer_includes_entry_actions_when_an_entry_is_selected() {
        let mut app = app_with_entry();
        app.focus = Focus::Entries;

        let text = footer_text(&app, true);

        assert!(text.contains("view (enter)"));
        assert!(text.contains("edit (e)"));
        assert!(text.contains("delete (d)"));
    }

    #[test]
    fn entries_footer_omits_entry_actions_without_a_selection() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let config = Config::new(dir.path().to_path_buf(), "true");
        let mut app = new_app(config);
        app.select_journal_by_name("work");
        app.focus = Focus::Entries;

        let text = footer_text(&app, true);

        assert!(!text.contains("view (enter)"));
        assert!(!text.contains("edit (e)"));
        assert!(!text.contains("delete (d)"));
    }

    #[test]
    fn search_results_footer_shows_escape_and_entry_actions() {
        let mut app = app_with_entry();
        app.mode = Mode::Search;
        app.focus = Focus::Entries;
        app.search.query = "body".to_string();
        app.search.hits = vec![crate::storage::SearchHit {
            path: app.entries[0].path.clone(),
            journal: "work".to_string(),
            title: "A".to_string(),
            preview: "Body".to_string(),
        }];

        let text = footer_text(&app, true);

        assert!(text.contains("Search all: body"));
        assert!(text.contains("view (enter)"));
        assert!(text.contains("exit search (esc)"));
        assert!(!text.contains("type query"));
        assert!(!text.contains("backspace"));
        assert!(!text.contains("edit (e)"));
        assert!(!text.contains("delete (d)"));
    }

    #[test]
    fn scoped_search_hit_labels_omit_journal_prefix() {
        let mut app = app_with_entry();
        app.search.scope = crate::tui::app::SearchScope::CurrentJournal("work".to_string());
        let hit = crate::storage::SearchHit {
            path: app.entries[0].path.clone(),
            journal: "work".to_string(),
            title: "A".to_string(),
            preview: "Body".to_string(),
        };

        assert_eq!(app.search_hit_label(&hit), "A");
    }

    #[test]
    fn global_search_hit_labels_include_journal_prefix() {
        let app = app_with_entry();
        let hit = crate::storage::SearchHit {
            path: app.entries[0].path.clone(),
            journal: "work".to_string(),
            title: "A".to_string(),
            preview: "Body".to_string(),
        };

        assert_eq!(app.search_hit_label(&hit), "work/A");
    }

    #[test]
    fn entry_list_lines_use_time_gutter_and_content() {
        let entry = Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from("id.md"),
            encryption_state: EntryEncryptionState::Plain,
            created_at: Some("2026-07-01T10:23:00+02:00".to_string()),
            updated_at: None,
            title: "Title".to_string(),
            preview: "Preview".to_string(),
            tags: Vec::new(),
            feelings: Vec::new(),
            mood: None,
            content: String::new(),
        };

        let lines = entry_list_lines(&entry, 30);
        let rendered: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "10:23  Title");
        assert_eq!(rendered[1], "       Preview");
    }

    #[test]
    fn entry_list_lines_wrap_long_title_onto_second_line() {
        let entry = Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from("id.md"),
            encryption_state: EntryEncryptionState::Plain,
            created_at: Some("2026-07-01T10:23:00+02:00".to_string()),
            updated_at: None,
            title: "A very long title".to_string(),
            preview: "preview text".to_string(),
            tags: Vec::new(),
            feelings: Vec::new(),
            mood: None,
            content: String::new(),
        };

        // text_width=12: "A very long title preview text" flows across three lines.
        // Line 1: "A very long" (break at space pos 11), line 2: "title", line 3: "preview text"
        let lines = entry_list_lines(&entry, 12);
        let rendered: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        assert_eq!(rendered.len(), 3);
        assert_eq!(rendered[0], "10:23  A very long");
        assert_eq!(rendered[1], "       title");
        assert_eq!(rendered[2], "       preview text");
    }

    #[test]
    fn locked_entry_list_lines_include_structural_marker() {
        let entry = Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from("work/2026/07/01/2026-07-01T10-23-00-id.md.age"),
            encryption_state: EntryEncryptionState::EncryptedLocked,
            created_at: None,
            updated_at: None,
            title: "[locked] Encrypted entry".to_string(),
            preview: "Encryption identity not available".to_string(),
            tags: Vec::new(),
            feelings: Vec::new(),
            mood: None,
            content: "Encryption identity not available".to_string(),
        };

        let lines = entry_list_lines(&entry, 100);
        let rendered: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "       [locked] Encrypted entry");
        assert_eq!(rendered[1], "       Encryption identity not available");
    }

    #[test]
    fn entry_group_labels_use_created_timestamp() {
        let entry = Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from("work/2026-01-01/id.md"),
            encryption_state: EntryEncryptionState::Plain,
            created_at: Some("2026-07-01T10:23:00+02:00".to_string()),
            updated_at: None,
            title: "Title".to_string(),
            preview: String::new(),
            tags: Vec::new(),
            feelings: Vec::new(),
            mood: None,
            content: String::new(),
        };

        assert_eq!(entry_month_label(&entry), Some("July 2026".to_string()));
        assert_eq!(entry_day_label(&entry), Some("Wednesday 01".to_string()));
    }

    #[test]
    fn entry_group_labels_fall_back_to_filename_date() {
        let entry = Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from("work/2026/07/01/2026-07-01T10-23-00-id.md"),
            encryption_state: EntryEncryptionState::Plain,
            created_at: None,
            updated_at: None,
            title: "Title".to_string(),
            preview: String::new(),
            tags: Vec::new(),
            feelings: Vec::new(),
            mood: None,
            content: String::new(),
        };

        assert_eq!(entry_month_label(&entry), Some("July 2026".to_string()));
        assert_eq!(entry_day_label(&entry), Some("Wednesday 01".to_string()));
    }
}
