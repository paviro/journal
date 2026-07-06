use super::*;
use crate::{
    config::Config,
    tui::{
        app::{Focus, INLINE_ENTRY_VIEW_MIN_WIDTH, Mode},
        state::{EditTagFocus, EditTagState, MetadataKind},
    },
};
use journal_storage::{Entry, EntryEncryptionState, JournalStore, SearchHit};
use ratatui::{Terminal, backend::TestBackend, layout::Rect, style::Modifier, text::Line};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use unicode_width::UnicodeWidthStr;

fn new_app(config: Config) -> App {
    let config_path = config.journal_root.join("config.toml");
    let store = JournalStore::for_config(&config_path, &config.journal_root).unwrap();
    App::new(config_path, config, store).unwrap()
}

fn app_with_entry() -> App {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let entry_dir = root.join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
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

fn metadata_values<'a>(
    tags: &'a [String],
    feelings: &'a [String],
    mood: Option<i8>,
) -> EntryMetadataValues<'a> {
    EntryMetadataValues {
        tags,
        people: &[],
        activities: &[],
        feelings,
        mood,
    }
}

fn render_confirm_delete_rows(width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            dialogs::draw_confirm_delete(
                frame,
                &crate::tui::state::DeleteContext::Entry { has_body: true },
            )
        })
        .unwrap();

    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

#[test]
fn layout_places_hit_targets_in_three_columns() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, 140, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.entry_view.is_some());
    assert!(layout.stats.is_none());
    assert_eq!(layout.journals.unwrap().area, Rect::new(0, 0, 22, 19));
    assert_eq!(layout.entries.unwrap().panel.area, Rect::new(22, 0, 42, 19));
    assert_eq!(layout.entry_view.unwrap().area, Rect::new(64, 0, 76, 19));
    assert_eq!(layout.footer, Rect::new(0, 19, 140, 1));
}

#[test]
fn layout_keeps_three_columns_at_minimum_inline_width() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, INLINE_ENTRY_VIEW_MIN_WIDTH, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.entry_view.is_some());
    assert!(layout.stats.is_none());
    let ch = 20 - footer_height(&app, INLINE_ENTRY_VIEW_MIN_WIDTH);
    assert_eq!(layout.journals.unwrap().area, Rect::new(0, 0, 22, ch));
    assert_eq!(layout.entries.unwrap().panel.area, Rect::new(22, 0, 42, ch));
    assert_eq!(layout.entry_view.unwrap().area, Rect::new(64, 0, 61, ch));
}

#[test]
fn layout_places_hit_targets_in_two_columns_without_inline_entry_view() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Journals;

    let layout = tui_layout(Rect::new(0, 0, 90, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.entry_view.is_none());
    assert!(layout.stats.is_none());
    assert_eq!(layout.journals.unwrap().area, Rect::new(0, 0, 22, 19));
    assert_eq!(layout.entries.unwrap().panel.area, Rect::new(22, 0, 68, 19));
}

#[test]
fn layout_shifts_two_columns_to_entries_and_preview_when_entries_are_active() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, 90, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.entry_view.is_some());
    assert!(layout.stats.is_none());
    assert!(layout.journals.is_none());
    let content_height = 20 - footer_height(&app, 90);
    assert_eq!(
        layout.entries.unwrap().panel.area,
        Rect::new(0, 0, 42, content_height)
    );
    assert_eq!(
        layout.entry_view.unwrap().area,
        Rect::new(42, 0, 48, content_height)
    );
}

#[test]
fn layout_uses_single_compact_panel_for_active_focus() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Journals;

    let journals = tui_layout(Rect::new(0, 0, 57, 20), &app);
    assert!(journals.single_panel);
    assert_eq!(
        journals.journals.unwrap().area,
        Rect::new(0, 0, 57, 20 - footer_height(&app, 57))
    );
    assert!(journals.entries.is_none());

    app.nav.focus = Focus::Entries;
    let entries = tui_layout(Rect::new(0, 0, 57, 20), &app);
    assert!(entries.single_panel);
    assert_eq!(
        entries.entries.unwrap().panel.area,
        Rect::new(0, 0, 57, 20 - footer_height(&app, 57))
    );
    assert!(entries.journals.is_none());
}

#[test]
fn entry_list_geometry_is_shared_by_render_hit_test_and_visibility() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;
    let layout = tui_layout(Rect::new(0, 0, 80, 20), &app);
    let entries = layout.entries.unwrap();

    assert_eq!(
        entries.text_width,
        entries.panel.content.width.saturating_sub(4)
    );

    let rows = entry_row_metadata(&app, entries.text_width);
    // Row 0 is the month divider; the single entry's box occupies rows 1-3.
    let click_y = entries.panel.content.y + 2;

    assert_eq!(
        entry_index_at(
            entries,
            entries.panel.content.x,
            click_y,
            app.nav.entry_list.offset(),
            &rows
        ),
        Some(0)
    );

    let offset_before = app.nav.entry_list.offset();
    app.entry_list_ensure_visible(&rows, entries.viewport_height);
    assert_eq!(app.nav.entry_list.offset(), offset_before);
}

#[test]
fn panel_content_rect_defines_selectable_rows_not_padding() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Journals;
    let layout = tui_layout(Rect::new(0, 0, 120, 20), &app);
    let journals = layout.journals.unwrap();

    // The first journal box sits one row below the content top (the leading
    // offset that aligns it with the entry list's first box).
    assert_eq!(
        journal_index_at(
            journals,
            journals.content.x,
            journals.content.y + 1,
            app.nav.journal_list.offset() as u16,
            app.library.journals.len()
        ),
        Some(0)
    );
    assert_eq!(
        journal_index_at(
            journals,
            panel_inner(journals.area).x,
            panel_inner(journals.area).y,
            app.nav.journal_list.offset() as u16,
            app.library.journals.len()
        ),
        None
    );
}

#[test]
fn metadata_hit_map_accounts_for_mood_row() {
    let area = Rect::new(42, 0, 60, 19);
    let tags = vec!["work".to_string()];
    let feelings = vec!["focused".to_string()];
    let values = metadata_values(&tags, &feelings, Some(2));
    let layout = crate::tui::surface::entry_metadata_layout(area, values);
    let feelings_row = layout.feelings.unwrap();
    let tags_row = layout.tags.unwrap();

    assert_eq!(
        metadata_at_point(
            area,
            feelings_row.rect.x + feelings_row.prefix_width,
            feelings_row.rect.y,
            values
        ),
        Some((MetadataChip::Feelings, "focused".to_string()))
    );
    assert_eq!(
        metadata_at_point(
            area,
            tags_row.rect.x + tags_row.prefix_width,
            tags_row.rect.y,
            values
        ),
        Some((MetadataChip::Tags, "work".to_string()))
    );
}

#[test]
fn metadata_hit_map_uses_terminal_cell_width_for_wide_text() {
    let area = Rect::new(42, 0, 60, 19);
    let tags = vec!["集中".to_string()];
    let feelings = vec!["嬉しい".to_string()];
    let values = metadata_values(&tags, &feelings, None);
    let layout = crate::tui::surface::entry_metadata_layout(area, values);
    let feelings_row = layout.feelings.unwrap();
    let tags_row = layout.tags.unwrap();

    assert_eq!(
        metadata_at_point(
            area,
            feelings_row.rect.x + feelings_row.prefix_width + 5,
            feelings_row.rect.y,
            values
        ),
        Some((MetadataChip::Feelings, "嬉しい".to_string()))
    );
    assert_eq!(
        metadata_at_point(
            area,
            tags_row.rect.x + tags_row.prefix_width + 3,
            tags_row.rect.y,
            values
        ),
        Some((MetadataChip::Tags, "集中".to_string()))
    );
}

#[test]
fn metadata_rows_wrap_without_leading_separator() {
    let values = vec![
        "calm".to_string(),
        "focused".to_string(),
        "tired".to_string(),
    ];

    let rows = crate::tui::surface::metadata_value_rows("Feelings: ".len() as u16, 20, &values);

    assert_eq!(rows, vec![vec![0], vec![1, 2]]);
}

#[test]
fn entry_view_wraps_metadata_rows_without_leading_space_or_separator() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
            entry_dir.join("a.md"),
            "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\ntags = [\"work\", \"personal\", \"health\"]\nfeelings = [\"calm\", \"focused\", \"tired\"]\n+++\n\n# A\nBody\n",
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf(), "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::EntryView;

    let tags = vec![
        "work".to_string(),
        "personal".to_string(),
        "health".to_string(),
    ];
    let feelings = vec![
        "calm".to_string(),
        "focused".to_string(),
        "tired".to_string(),
    ];
    let entry_view = Rect::new(0, 0, 24, 60 - expanded_footer_height(&app, 24));
    let values = metadata_values(&tags, &feelings, None);
    let metadata = crate::tui::surface::entry_metadata_layout(entry_view, values);
    let feelings_row = metadata.feelings.unwrap();
    let tags_row = metadata.tags.unwrap();

    let backend = render_app(app, 24, 60);
    let buffer = backend.buffer();

    assert_eq!(feelings_row.rect.height, 2);
    assert_eq!(tags_row.rect.height, 2);
    assert_eq!(
        buffer
            .cell((feelings_row.rect.x, feelings_row.rect.y + 1))
            .unwrap()
            .symbol(),
        "f"
    );
    assert_eq!(
        buffer
            .cell((tags_row.rect.x, tags_row.rect.y + 1))
            .unwrap()
            .symbol(),
        "p"
    );
    assert_eq!(
        metadata_at_point(
            entry_view,
            feelings_row.rect.x,
            feelings_row.rect.y + 1,
            values
        ),
        Some((MetadataChip::Feelings, "focused".to_string()))
    );
    assert_eq!(
        metadata_at_point(entry_view, tags_row.rect.x, tags_row.rect.y + 1, values),
        Some((MetadataChip::Tags, "personal".to_string()))
    );
}

#[test]
fn short_entry_view_scrolls_metadata_after_body() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    let body = (1..=40)
        .map(|index| format!("Line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
            entry_dir.join("a.md"),
            format!(
                "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\ntags = [\"tiny-screen\"]\nfeelings = [\"focused\"]\n+++\n\n# A\n{body}\n",
            ),
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf(), "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::EntryView;

    let top = render_text(app, 80, 20);
    assert!(!top.contains("Tags: tiny-screen"));

    let mut app = new_app(Config::new(dir.path().to_path_buf(), "true"));
    app.select_journal_by_name("work");
    app.nav.focus = Focus::EntryView;
    app.nav.scroll.entry_view = u16::MAX;

    let bottom = render_text(app, 80, 20);
    assert!(bottom.contains("Feelings: focused"));
    assert!(bottom.contains("Tags: tiny-screen"));
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
    assert_eq!(scrollbar_position(scroll as usize, line_count, height), 39);
}

#[test]
fn scrollbar_position_stays_at_start_when_content_fits() {
    assert_eq!(scrollbar_position(0, 4, 8), 0);
}

#[test]
fn scrollbar_bar_rect_matches_rendered_track() {
    // Panel at (2, 3) sized 20×10: bar on the rightmost column, inset one row
    // top and bottom (Margin { vertical: 1 }).
    let bar = scrollbar_bar_rect(Rect::new(2, 3, 20, 10));
    assert_eq!(bar, Rect::new(21, 4, 1, 8));
}

#[test]
fn scroll_from_thumb_top_maps_travel_ends_to_scroll_range() {
    // track_top 5, track_len 10, thumb_len 4 → the thumb travels rows 5..=11
    // (travel 6). Top of travel → 0, bottom → max, rows clamp past the ends.
    let (track_top, track_len, thumb_len, max) = (5, 10, 4, 100);
    assert_eq!(
        scroll_from_thumb_top(track_top, track_top, track_len, thumb_len, max),
        0
    );
    assert_eq!(
        scroll_from_thumb_top(0, track_top, track_len, thumb_len, max),
        0
    );
    assert_eq!(
        scroll_from_thumb_top(track_top + 6, track_top, track_len, thumb_len, max),
        max
    );
    assert_eq!(
        scroll_from_thumb_top(u16::MAX, track_top, track_len, thumb_len, max),
        max
    );
}

#[test]
fn scroll_from_thumb_top_handles_untravellable_thumbs() {
    assert_eq!(scroll_from_thumb_top(7, 5, 10, 4, 0), 0); // no overflow
    assert_eq!(scroll_from_thumb_top(7, 5, 4, 4, 100), 0); // thumb fills track
}

#[test]
fn scrollbar_thumb_sits_below_the_up_arrow_at_the_top() {
    // Bar of 12 rows starting at y=3: arrows at rows 3 and 14, track rows 4..=13.
    let bar = Rect::new(20, 3, 1, 12);
    let (top, len) = scrollbar_thumb(bar, 40, 10, 0).expect("thumb");
    assert_eq!(top, 4, "thumb starts just below the up arrow at scroll 0");
    assert!(len >= 1);
}

#[test]
fn scrollbar_thumb_reaches_bottom_of_track_at_max_scroll() {
    let bar = Rect::new(20, 3, 1, 12);
    let line_count = 40;
    let height = 10;
    let scroll = viewer_scroll(u16::MAX, line_count, height) as usize;
    let position = scrollbar_position(scroll, line_count, height);
    let (top, len) = scrollbar_thumb(bar, line_count, height, position).expect("thumb");
    // Track rows are 4..=13; the thumb's bottom edge reaches the last track row.
    assert_eq!(top + len - 1, 13);
}

#[test]
fn scrollbar_thumb_none_when_bar_too_short() {
    assert_eq!(scrollbar_thumb(Rect::new(0, 0, 1, 2), 40, 10, 0), None);
}

#[test]
fn list_dialogs_keep_preferred_width_until_they_hit_edges() {
    let wide_tags = tags_dialog_layout(Rect::new(0, 0, 120, 30), 20);
    assert_eq!(wide_tags.area.width, 44);
    assert_eq!(wide_tags.list.height, 14);

    let narrow_tags = tags_dialog_layout(Rect::new(0, 0, 40, 30), 20);
    assert_eq!(narrow_tags.area.x, 0);
    assert_eq!(narrow_tags.area.width, 40);

    let wide_feelings = feelings_dialog_layout(Rect::new(0, 0, 120, 30), 24);
    assert_eq!(wide_feelings.area.width, 44);
    assert_eq!(wide_feelings.list.height, 16);

    let wide_mood = mood_dialog_layout(Rect::new(0, 0, 120, 30));
    assert_eq!(wide_mood.area.width, 90);

    let narrow_mood = mood_dialog_layout(Rect::new(0, 0, 80, 30));
    assert_eq!(narrow_mood.area.x, 0);
    assert_eq!(narrow_mood.area.width, 80);
}

#[test]
fn confirm_delete_message_is_centered_in_dialog_body() {
    let rows = render_confirm_delete_rows(80, 20);
    let message_row = rows
        .iter()
        .position(|row| row.contains("Move entry to trash?  y/n"))
        .unwrap();
    let title_row = rows
        .iter()
        .position(|row| row.contains("Confirm Delete"))
        .unwrap();

    assert_eq!(message_row, title_row + 2);
}

#[test]
fn edit_tags_dialog_keeps_help_visible_below_spacer() {
    let all_tags: Vec<(String, usize)> = (0..20)
        .map(|index| (format!("tag-{index:02}"), index))
        .collect();
    let filtered: Vec<usize> = (0..all_tags.len()).collect();
    let rendered = render_edit_tags_dialog_text(
        EditTagState::new(MetadataKind::Tags, all_tags, filtered, Vec::new()),
        200,
        20,
    );

    assert!(rendered.contains(">[ ] tag-00 (0)"));
    assert!(rendered.contains("toggle (space)"));
    assert!(rendered.contains("input (tab)"));
    assert!(rendered.contains("save (enter)"));
    assert!(rendered.contains("cancel (esc)"));
}

#[test]
fn edit_tags_dialog_keeps_list_gutter_when_selection_is_scrolled_out() {
    let all_tags: Vec<(String, usize)> = (0..20)
        .map(|index| (format!("tag-{index:02}"), index))
        .collect();
    let filtered: Vec<usize> = (0..all_tags.len()).collect();
    let mut state = EditTagState::new(MetadataKind::Tags, all_tags, filtered, Vec::new());
    state.list.set_offset(5);

    let rendered = render_edit_tags_dialog_text(state, 200, 20);

    assert!(rendered.contains(" [ ] tag-05 (5)"));
}

#[test]
fn edit_tags_dialog_counts_no_matches_row_when_sizing() {
    let mut state = EditTagState::new(
        MetadataKind::Tags,
        vec![("work".to_string(), 1)],
        Vec::new(),
        Vec::new(),
    );
    state.input = "missing".to_string();
    state.focus = EditTagFocus::Input;
    let rendered = render_edit_tags_dialog_text(state, 200, 12);

    assert!(rendered.contains(" (no matches)"));
    assert!(rendered.contains(" add (enter) | list (tab) | cancel (esc)"));
}

#[test]
fn edit_metadata_input_hint_saves_when_empty_and_adds_when_not_empty() {
    let mut empty = EditTagState::new(MetadataKind::People, Vec::new(), Vec::new(), Vec::new());
    empty.focus = EditTagFocus::Input;
    let rendered_empty = render_edit_tags_dialog_text(empty, 200, 12);
    assert!(rendered_empty.contains(" save (enter) | list (tab) | cancel (esc)"));

    let mut with_value =
        EditTagState::new(MetadataKind::People, Vec::new(), Vec::new(), Vec::new());
    with_value.focus = EditTagFocus::Input;
    with_value.input = "alex".to_string();
    let rendered_value = render_edit_tags_dialog_text(with_value, 200, 12);
    assert!(rendered_value.contains(" add (enter) | list (tab) | cancel (esc)"));
}

#[test]
fn entry_hit_testing_ignores_month_divider_and_maps_boxed_entries() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nFirst preview\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nSecond preview\n",
    )
    .unwrap();
    let config = Config::new(dir.path().to_path_buf(), "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    let area = EntryListGeometry::new(Rect::new(0, 0, 40, 16));
    // text_width=10 wraps each preview onto 2 lines, so a box is 4 rows tall
    // (top border + 2 preview lines + bottom border). A single month divider
    // row leads the list; the day rides on the first entry's border, and a
    // blank spacer row separates consecutive entries.
    let rows = entry_row_metadata(&app, 10);

    assert_eq!(
        rows,
        vec![
            EntryRowMeta {
                entry_index: None,
                height: 1,
            },
            EntryRowMeta {
                entry_index: Some(0),
                height: 4,
            },
            EntryRowMeta {
                entry_index: None,
                height: 1,
            },
            EntryRowMeta {
                entry_index: Some(1),
                height: 4,
            },
        ]
    );
    // Rows: month divider (y 1), entry 0 (y 2-5), spacer (y 6), entry 1 (y 7-10).
    assert_eq!(entry_index_at(area, 2, 1, 0, &rows), None);
    assert_eq!(entry_index_at(area, 2, 2, 0, &rows), Some(0));
    assert_eq!(entry_index_at(area, 2, 5, 0, &rows), Some(0));
    assert_eq!(entry_index_at(area, 2, 6, 0, &rows), None);
    assert_eq!(entry_index_at(area, 2, 7, 0, &rows), Some(1));
    assert_eq!(entry_index_at(area, 2, 10, 0, &rows), Some(1));
}

#[test]
fn first_month_rides_border_and_next_month_takes_over_after_scrolling() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    // Two July entries (newest, listed first) over many June entries. The
    // June entries give the list a viewport-full of rows below the June
    // divider so it can actually be scrolled above the top.
    let mut days = vec![("2026-07-02", "2026-07-02T10:00:00+02:00")];
    days.push(("2026-07-01", "2026-07-01T10:00:00+02:00"));
    for day in 1..=10 {
        days.push((
            Box::leak(format!("2026-06-{day:02}").into_boxed_str()),
            Box::leak(format!("2026-06-{day:02}T10:00:00+02:00").into_boxed_str()),
        ));
    }
    for (index, (dir_day, ts)) in days.iter().enumerate() {
        let entry_dir = root.join("work").join(dir_day);
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join(format!("e{index}.md")),
            format!("+++\ncreated_at = \"{ts}\"\n+++\n\n# e{index}\nBody text\n"),
        )
        .unwrap();
    }

    // Before scrolling, the first month (July) already rides the border and
    // its divider is absent from the list body (row 0 is the leading blank).
    let top_unscrolled = render_top_border(app_for(&dir), 57, 12);
    assert!(top_unscrolled.contains("July 2026"), "{top_unscrolled:?}");

    // Scroll far enough that the June divider clears the top; June takes over.
    let mut app = app_for(&dir);
    *app.nav.entry_list.offset_mut() = 100;
    let backend = render_app(app, 57, 12);
    let top = (0..57)
        .map(|x| backend.buffer().cell((x, 0)).unwrap().symbol().to_string())
        .collect::<String>();
    assert!(top.contains("June 2026"), "top border was: {top:?}");
}

fn app_for(dir: &tempfile::TempDir) -> App {
    let mut app = new_app(Config::new(dir.path().to_path_buf(), "true"));
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;
    app
}

fn render_top_border(app: App, width: u16, height: u16) -> String {
    let backend = render_app(app, width, height);
    (0..width)
        .map(|x| backend.buffer().cell((x, 0)).unwrap().symbol().to_string())
        .collect()
}

#[test]
fn entry_view_renders_feelings_metadata() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
            entry_dir.join("a.md"),
            "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\nfeelings = [\"calm\", \"focused\"]\n+++\n\n# A\nBody\n",
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf(), "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::EntryView;

    let rendered = render_text(app, 120, 20);

    assert!(rendered.contains("Feelings: calm | focused"));
}

#[test]
fn entry_view_renders_indented_mermaid_diagram() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
            entry_dir.join("a.md"),
            "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\n```mermaid\n  graph TD\n      A[Open journal] --> B[Write entry]\n      B --> C{Preview}\n      C -->|looks good| D[Save]\n      C -->|needs work| B\n  ```\n",
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf(), "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::EntryView;

    let rendered = render_text(app, 140, 28);

    assert!(rendered.contains("mermaid"));
    assert!(rendered.contains("Open journal"));
    assert!(rendered.contains("Write entry"));
}

#[test]
fn list_panels_show_counts_in_bottom_titles() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let work_entry_dir = root.join("work").join("2026-07-01");
    fs::create_dir_all(&work_entry_dir).unwrap();
    fs::write(
        work_entry_dir.join("a.md"),
        "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    fs::write(
        work_entry_dir.join("b.md"),
        "+++\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nBody\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("personal")).unwrap();

    let config = Config::new(root, "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;

    let rendered = render_text(app, 130, 20);

    assert!(rendered.contains("2 journals"));
    assert!(rendered.contains("2 entries"));
}

#[test]
fn compact_render_shows_only_the_active_step() {
    let mut journals_app = app_with_entry();
    journals_app.nav.focus = Focus::Journals;
    let journals = render_text(journals_app, 57, 16);
    assert!(journals.contains(" Journals "));
    assert!(!journals.contains(" Entries "));
    assert!(!journals.contains("2026-07-01 10:00"));

    let mut entries_app = app_with_entry();
    entries_app.nav.focus = Focus::Entries;
    let entries = render_text(entries_app, 57, 16);
    assert!(entries.contains(" Entries "));
    assert!(!entries.contains(" Journals "));
    assert!(!entries.contains("2026-07-01 10:00"));

    let mut entry_view_focus_app = app_with_entry();
    entry_view_focus_app.nav.focus = Focus::EntryView;
    let entry_view_focus = render_text(entry_view_focus_app, 57, 16);
    assert!(!entry_view_focus.contains(" Entries "));
    assert!(!entry_view_focus.contains(" Journals "));
    assert!(entry_view_focus.contains("Body"));
}

#[test]
fn two_column_render_follows_active_column_pair() {
    let mut journals_app = app_with_entry();
    journals_app.nav.focus = Focus::Journals;
    let journals = render_text(journals_app, 90, 16);
    assert!(journals.contains(" Journals "));
    assert!(journals.contains(" Entries "));
    assert!(!journals.contains("2026-07-01 10:00"));

    let mut entries_app = app_with_entry();
    entries_app.nav.focus = Focus::Entries;
    let entries = render_text(entries_app, 90, 16);
    assert!(entries.contains(" Entries "));
    assert!(!entries.contains(" Journals "));
    assert!(entries.contains("Wednesday, 1 July 2026, 10:00"));
}

#[test]
fn selected_journal_and_entry_remain_reversed_when_entry_view_is_focused() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::EntryView;

    let backend = render_app(app, 130, 20);
    let buffer = backend.buffer();

    // Journal box 0 spans rows 2-4 (after the leading offset); its inside is
    // reversed while selected.
    assert!(
        buffer
            .cell((2, 3))
            .unwrap()
            .modifier
            .contains(Modifier::REVERSED)
    );
    assert!(
        buffer
            .cell((24, 3))
            .unwrap()
            .modifier
            .contains(Modifier::REVERSED)
    );
}

#[test]
fn selected_entry_is_not_reversed_when_journals_are_focused() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Journals;

    let backend = render_app(app, 120, 20);
    let buffer = backend.buffer();

    // The selected journal box (rows 2-4) is reversed, but no entry in the
    // entries column is, since journals hold focus.
    assert!(
        buffer
            .cell((2, 3))
            .unwrap()
            .modifier
            .contains(Modifier::REVERSED)
    );
    assert!(
        !buffer
            .cell((24, 3))
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
    app.nav.focus = Focus::Journals;

    let text = footer_text(&app);

    assert!(!text.contains("view (enter)"));
    assert!(!text.contains("edit (e)"));
    assert!(!text.contains("del (d)"));
}

#[test]
fn entries_footer_includes_entry_actions_when_an_entry_is_selected() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let text = footer_text(&app);

    assert!(text.contains("view (enter)"));
    assert!(text.contains("edit (e)"));
    assert!(text.contains("del (d)"));
}

#[test]
fn expanded_entry_footer_includes_inline_entry_actions() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::EntryView;

    let inline_text = footer_text(&app);
    let expanded_text = expanded_footer_text(&app);

    for label in [
        "new entry (n)",
        "edit (e)",
        "del (d)",
        "tags (t)",
        "feel (f)",
        "mood (m)",
        "search (/)",
        "quit (q)",
    ] {
        assert!(inline_text.contains(label));
        assert!(expanded_text.contains(label));
    }
    for label in ["ppl (p)", "act (a)"] {
        assert!(!inline_text.contains(label));
        assert!(expanded_text.contains(label));
    }
    assert!(expanded_text.contains("close (enter/esc)"));
    assert!(expanded_text.contains("edit (e) | close (enter/esc) | del (d)"));
}

#[test]
fn expanded_entry_draws_confirm_delete_overlay() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::EntryView;
    app.begin_confirm_delete();

    let text = render_text(app, 80, 20);

    assert!(text.contains("Confirm Delete"));
    assert!(text.contains("Move entry to trash?  y/n"));
}

#[test]
fn entries_footer_omits_entry_actions_without_a_selection() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("work")).unwrap();
    let config = Config::new(dir.path().to_path_buf(), "true");
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;

    let text = footer_text(&app);

    assert!(!text.contains("view (enter)"));
    assert!(!text.contains("edit (e)"));
    assert!(!text.contains("del (d)"));
}

#[test]
fn search_results_footer_shows_escape_and_entry_actions() {
    let mut app = app_with_entry();
    app.nav.mode = Mode::Search;
    app.nav.focus = Focus::Entries;
    app.search.query = "body".to_string();
    app.search.hits = vec![SearchHit {
        id: app.library.entries[0].id.clone(),
        journal: "work".to_string(),
        created_at: None,
        title: "A".to_string(),
        preview: "Body".to_string(),
    }];

    let text = footer_text(&app);

    // The query now lives on the entry panel's top-right border, not the footer.
    assert!(!text.contains("Search all: body"));
    assert!(text.contains("view (enter)"));
    assert!(text.contains("exit search (esc)"));
    assert!(!text.contains("type query"));
    assert!(!text.contains("backspace"));
    assert!(!text.contains("edit (e)"));
    assert!(!text.contains("del (d)"));
}

#[test]
fn narrow_footer_wraps_actions_below_columns() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, 60, 20), &app);

    assert!(layout.footer.height > 1);
    assert_eq!(layout.footer.height, footer_height(&app, 60));
    assert_eq!(layout.content.height, 20 - layout.footer.height);
}

#[test]
fn wrapped_footer_hint_routing_uses_visible_row() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    assert_eq!(
        footer_hint_id_at_point(&app, 0, 18, 60, 0, 19),
        Some(HintId::BeginEditFeelings)
    );
}

#[test]
fn footer_hint_routing_uses_typed_ids() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;
    let text = footer_text(&app);

    assert_eq!(
        footer_hint_id_at(&app, 0, text.find("tags (t)").unwrap() as u16),
        Some(HintId::BeginEditTags)
    );
    assert_eq!(
        footer_hint_id_at(&app, 0, text.find("edit (e)").unwrap() as u16),
        Some(HintId::EditSelected)
    );
}

#[test]
fn expanded_footer_hint_routing_uses_typed_ids() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::EntryView;
    let text = expanded_footer_text(&app);

    assert_eq!(
        expanded_footer_hint_id_at_point(
            &app,
            0,
            19,
            120,
            1 + text.find("tags (t)").unwrap() as u16,
            19
        ),
        Some(HintId::BeginEditTags)
    );
}

#[test]
fn dialog_hints_wrap_and_remain_clickable_by_row() {
    let hints = tags_dialog_hints(EditTagFocus::List, true);

    assert_eq!(hint_height(hints, 29), 2);
    assert_eq!(
        hint_id_at_wrapped(hints, 10, 5, 29, 10, 6),
        Some(HintId::TagsSave)
    );
}

#[test]
fn dialog_hint_routing_uses_typed_ids() {
    let tags = tags_dialog_hints(EditTagFocus::List, true);
    assert_eq!(hint_id_at(tags, 10, 11), Some(HintId::TagsToggle));

    let empty_input = tags_dialog_hints(EditTagFocus::Input, true);
    assert_eq!(hint_id_at(empty_input, 10, 11), Some(HintId::TagsSave));

    let value_input = tags_dialog_hints(EditTagFocus::Input, false);
    assert_eq!(
        hint_id_at(value_input, 10, 11),
        Some(HintId::TagsAddFromInput)
    );

    let feelings = feelings_dialog_hints();
    assert_eq!(
        hint_id_at(feelings, 20, 20 + "toggle (space) | ".len() as u16),
        Some(HintId::FeelingsSave)
    );

    let mood = mood_dialog_hints();
    assert_eq!(
        hint_id_at(
            mood,
            30,
            30 + UnicodeWidthStr::width("decrease (←) | ") as u16
        ),
        Some(HintId::MoodIncrease)
    );
}

#[test]
fn scoped_search_hit_labels_omit_journal_prefix() {
    let mut app = app_with_entry();
    app.search.scope = crate::tui::app::SearchScope::Journal("work".to_string());
    let hit = SearchHit {
        id: app.library.entries[0].id.clone(),
        journal: "work".to_string(),
        created_at: None,
        title: "A".to_string(),
        preview: "Body".to_string(),
    };

    assert_eq!(app.search_hit_label(&hit), "A");
}

#[test]
fn global_search_hit_labels_include_journal_prefix() {
    let app = app_with_entry();
    let hit = SearchHit {
        id: app.library.entries[0].id.clone(),
        journal: "work".to_string(),
        created_at: None,
        title: "A".to_string(),
        preview: "Body".to_string(),
    };

    assert_eq!(app.search_hit_label(&hit), "work/A");
}

fn rendered_lines(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

fn plain_entry(created_at: Option<&str>, preview: &str) -> Entry {
    Entry {
        id: "id".to_string(),
        journal: "work".to_string(),
        path: PathBuf::from("id.md"),
        encryption_state: EntryEncryptionState::Plain,
        created_at: created_at.map(journal_storage::Timestamp::parse),
        updated_at: None,
        preview: preview.to_string(),
        metadata: journal_storage::Metadata::default(),
        import_id: None,
        content: String::new(),
        word_count: 0,
        search_haystack: String::new(),
    }
}

#[test]
fn entry_list_lines_put_time_on_right_of_border() {
    let entry = plain_entry(Some("2026-07-01T10:23:00+02:00"), "Preview");

    let rendered = rendered_lines(&entry_list_lines(&entry, None, 30));

    assert_eq!(rendered.len(), 3);
    // No date on the first line here, so the time sits alone on the right.
    assert!(rendered[0].starts_with('┌'));
    assert!(rendered[0].ends_with("10:23 ┐"));
    assert!(!rendered[0].contains('·'));
    assert!(rendered[1].starts_with("│ Preview"));
    assert!(rendered[1].ends_with('│'));
    assert!(rendered[2].starts_with('└'));
    assert!(rendered[2].ends_with('┘'));
}

#[test]
fn entry_list_lines_put_day_left_and_time_right() {
    let entry = plain_entry(Some("2026-07-05T14:30:00+02:00"), "Body");

    let rendered = rendered_lines(&entry_list_lines(&entry, Some("Sunday 05"), 30));

    assert!(rendered[0].starts_with("┌ Sunday 05 "));
    assert!(rendered[0].ends_with("14:30 ┐"));
    assert!(!rendered[0].contains('·'));
}

#[test]
fn entry_box_lines_without_timestamp_render_plain_top_border() {
    let rendered = rendered_lines(&entry_box_lines(None, "", "just a preview", None, 30));

    assert_eq!(rendered[0], format!("┌{}┐", "─".repeat(32)));
    assert!(rendered[1].starts_with("│ just a preview"));
}

#[test]
fn search_hit_box_shows_date_time_and_journal() {
    let rendered = rendered_lines(&entry_box_lines(
        Some("Sun 05 Jul 2026"),
        "14:30",
        "hit body",
        Some("work"),
        30,
    ));

    assert!(rendered[0].starts_with("┌ Sun 05 Jul 2026 "));
    assert!(rendered[0].ends_with("14:30 ┐"));
    assert!(rendered[1].starts_with("│ hit body"));
    // Journal on the bottom-left.
    assert!(rendered.last().unwrap().starts_with("└ work "));
}

#[test]
fn entry_group_labels_use_created_timestamp() {
    let entry = Entry {
        id: "id".to_string(),
        journal: "work".to_string(),
        path: PathBuf::from("work/2026-01-01/id.md"),
        encryption_state: EntryEncryptionState::Plain,
        created_at: Some(journal_storage::Timestamp::parse(
            "2026-07-01T10:23:00+02:00",
        )),
        updated_at: None,
        preview: String::new(),
        metadata: journal_storage::Metadata::default(),
        import_id: None,
        content: String::new(),
        word_count: 0,
        search_haystack: String::new(),
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
        preview: String::new(),
        metadata: journal_storage::Metadata::default(),
        import_id: None,
        content: String::new(),
        word_count: 0,
        search_haystack: String::new(),
    };

    assert_eq!(entry_month_label(&entry), Some("July 2026".to_string()));
    assert_eq!(entry_day_label(&entry), Some("Wednesday 01".to_string()));
}
