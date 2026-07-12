use super::*;
use crate::{
    config::Config,
    tui::{
        app::{EditMetadataFocus, EditMetadataState, Focus, INLINE_READER_MIN_WIDTH, Mode},
        state::MetadataKind,
        test_support::{app_with_entries, app_with_entry, app_with_journals, new_app},
        theme,
    },
};
use notema_domain::{Entry, EntryEncryptionState, SearchHit};
use ratatui::{Frame, Terminal, backend::TestBackend, layout::Rect, style::Modifier, text::Line};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

/// Draw `draw` onto a fresh `width`×`height` test terminal and return the
/// backend, the shared plumbing behind the typed render helpers below.
fn render_backend(width: u16, height: u16, draw: impl FnOnce(&mut Frame)) -> TestBackend {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(draw).unwrap();
    terminal.backend().clone()
}

/// The rendered buffer as one flat string (every cell symbol, row by row).
fn render_to_text(width: u16, height: u16, draw: impl FnOnce(&mut Frame)) -> String {
    render_backend(width, height, draw)
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

/// The rendered buffer split into one string per row.
fn render_to_rows(width: u16, height: u16, draw: impl FnOnce(&mut Frame)) -> Vec<String> {
    render_backend(width, height, draw)
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn render_text(mut app: App, width: u16, height: u16) -> String {
    render_to_text(width, height, |frame| draw(frame, &mut app))
}

fn render_app(mut app: App, width: u16, height: u16) -> TestBackend {
    render_backend(width, height, |frame| draw(frame, &mut app))
}

fn render_edit_tags_dialog_text(mut state: EditMetadataState, width: u16, height: u16) -> String {
    render_to_text(width, height, |frame| {
        dialogs::draw_edit_metadata_dialog(frame, &mut state, crate::tui::state::HoverTarget::None)
    })
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
        location: None,
    }
}

fn render_confirm_delete_rows(width: u16, height: u16) -> Vec<String> {
    render_to_rows(width, height, |frame| {
        dialogs::draw_confirm_delete(
            frame,
            &crate::tui::state::DeleteContext::Entry { has_body: true },
            crate::tui::state::HoverTarget::None,
        )
    })
}

#[test]
fn layout_places_hit_targets_in_three_columns() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, 140, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.reader.is_some());
    assert!(layout.insights.is_none());
    // The three columns share the rows the footer doesn't take.
    let footer_h = footer_height(&app, 140);
    let content_h = 20 - footer_h;
    assert_eq!(
        layout.journals.unwrap().area,
        Rect::new(0, 0, 27, content_h)
    );
    assert_eq!(
        layout.entries.unwrap().panel.area,
        Rect::new(27, 0, 47, content_h)
    );
    assert_eq!(layout.reader.unwrap().area, Rect::new(74, 0, 66, content_h));
    assert_eq!(layout.footer, Rect::new(0, content_h, 140, footer_h));
}

#[test]
fn layout_keeps_three_columns_at_minimum_inline_width() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, INLINE_READER_MIN_WIDTH, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.reader.is_some());
    assert!(layout.insights.is_none());
    let ch = 20 - footer_height(&app, INLINE_READER_MIN_WIDTH);
    assert_eq!(layout.journals.unwrap().area, Rect::new(0, 0, 27, ch));
    assert_eq!(layout.entries.unwrap().panel.area, Rect::new(27, 0, 47, ch));
    assert_eq!(layout.reader.unwrap().area, Rect::new(74, 0, 51, ch));
}

#[test]
fn layout_places_hit_targets_in_two_columns_without_inline_reader() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Journals;

    let layout = tui_layout(Rect::new(0, 0, 90, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.reader.is_none());
    assert!(layout.insights.is_none());
    let ch = 20 - footer_height(&app, 90);
    assert_eq!(layout.journals.unwrap().area, Rect::new(0, 0, 27, ch));
    assert_eq!(layout.entries.unwrap().panel.area, Rect::new(27, 0, 63, ch));
}

#[test]
fn layout_shifts_two_columns_to_entries_and_reader_when_entries_are_active() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, 90, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.reader.is_some());
    assert!(layout.insights.is_none());
    assert!(layout.journals.is_none());
    let content_height = 20 - footer_height(&app, 90);
    assert_eq!(
        layout.entries.unwrap().panel.area,
        Rect::new(0, 0, 47, content_height)
    );
    assert_eq!(
        layout.reader.unwrap().area,
        Rect::new(47, 0, 43, content_height)
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
    let list_area = journal_list_rect(journals.content);
    let inner_width = list_area.width.saturating_sub(4) as usize;
    let rows = crate::tui::entry_rows::journal_list_rows(&app, inner_width);
    let meta = crate::tui::entry_rows::rows_meta(&rows);

    // The first journal box sits one row below the content top (the leading
    // offset that aligns it with the entry list's first box).
    assert_eq!(
        journal_index_at(
            journals.content,
            journals.content.x,
            journals.content.y + 1,
            app.nav.journal_list.offset(),
            &meta,
        ),
        Some(0)
    );
    assert_eq!(
        journal_index_at(
            journals.content,
            panel_inner(journals.area).x,
            panel_inner(journals.area).y,
            app.nav.journal_list.offset(),
            &meta,
        ),
        None
    );
}

#[test]
fn journal_column_inserts_archived_divider_between_sections() {
    let app = app_with_journals(&["work", "zeta", "old.archived"]);
    let rows = crate::tui::entry_rows::journal_list_rows(&app, 16);
    let meta = crate::tui::entry_rows::rows_meta(&rows);

    // Two active journals, then a non-selectable divider row, then the archived one.
    let indices: Vec<Option<usize>> = meta.iter().map(|m| m.item_index).collect();
    assert_eq!(indices, vec![Some(0), Some(1), None, Some(2)]);

    // The rendered column carries the "Archived" divider and the archived
    // journal's display name (no ".archived" suffix leaks into the UI).
    let text = render_text(app, 120, 24);
    assert!(text.contains("Archived"));
    assert!(!text.contains(".archived"));
    // The panel count includes the archived journal (2 active + 1 archived).
    assert!(text.contains("3 journals"));
}

#[test]
fn journal_column_has_no_divider_without_archived_journals() {
    let app = app_with_journals(&["work", "zeta"]);
    let rows = crate::tui::entry_rows::journal_list_rows(&app, 16);
    let meta = crate::tui::entry_rows::rows_meta(&rows);
    assert!(meta.iter().all(|m| m.item_index.is_some()));
}

#[test]
fn search_hit_box_flags_archived_journal_bottom_right() {
    let rendered = rendered_lines(&entry_box_lines(
        Some("Sun 05 Jul 2026"),
        "14:30",
        "hit body",
        Some("personal"),
        Some("Archived"),
        40,
    ));
    let bottom = rendered.last().unwrap();
    // Journal display name on the left, the `Archived` flag on the right.
    assert!(bottom.starts_with("└ personal "));
    assert!(bottom.ends_with("Archived ┘"));
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
fn metadata_layout_places_location_row_after_tags() {
    let area = Rect::new(42, 0, 60, 19);
    let tags = vec!["work".to_string()];
    let values = EntryMetadataValues {
        tags: &tags,
        people: &[],
        activities: &[],
        feelings: &[],
        mood: None,
        location: Some("Testville, Testland"),
    };

    let layout = crate::tui::surface::entry_metadata_layout(area, values);
    let tags_row = layout.tags.unwrap();
    let location_row = layout.location.expect("location row is laid out");

    // Stacked below tags, and it does not participate in the click hit-test.
    assert!(location_row.rect.y >= tags_row.rect.y + tags_row.rect.height);
    assert_eq!(
        metadata_at_point(
            area,
            location_row.rect.x + location_row.prefix_width,
            location_row.rect.y,
            values
        ),
        None
    );
}

#[test]
fn location_wrapped_lines_break_a_long_label_flush_left() {
    let label = "Cafe Central - Main Street, Inner City, 1010 Vienna, Austria";
    let lines = crate::tui::surface::location_wrapped_lines(10, 30, label);

    assert!(lines.len() >= 2, "long label should wrap: {lines:?}");
    // First line leaves room for the 10-cell "Location: " prefix; the rest use
    // the full width. No continuation line starts with a leading space.
    assert!(crate::tui::entry_rows::text_width(&lines[0]) <= 20);
    for line in &lines[1..] {
        assert!(crate::tui::entry_rows::text_width(line) <= 30);
        assert!(!line.starts_with(' '));
    }
}

#[test]
fn location_row_height_reflects_wrapped_lines() {
    let area = Rect::new(0, 0, 24, 60);

    let short_layout = crate::tui::surface::entry_metadata_layout(
        area,
        EntryMetadataValues {
            tags: &[],
            people: &[],
            activities: &[],
            feelings: &[],
            mood: None,
            location: Some("Cafe"),
        },
    );
    let long_layout = crate::tui::surface::entry_metadata_layout(
        area,
        EntryMetadataValues {
            tags: &[],
            people: &[],
            activities: &[],
            feelings: &[],
            mood: None,
            location: Some("Grand Central Station Cafe"),
        },
    );

    assert_eq!(short_layout.location.unwrap().rect.height, 1);
    assert!(long_layout.location.unwrap().rect.height >= 2);
}

#[test]
fn reader_wraps_long_location_with_flush_left_continuation() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
            entry_dir.join("a.md"),
            "+++\nschema_version = 1\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n\n[location]\nname = \"Grand Central Station Cafe\"\n+++\n\n# A\nBody\n",
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Reader;

    let reader = Rect::new(0, 0, 24, 60 - expanded_footer_height(&app, 24));
    let values = EntryMetadataValues {
        tags: &[],
        people: &[],
        activities: &[],
        feelings: &[],
        mood: None,
        location: Some("Grand Central Station Cafe"),
    };
    let metadata = crate::tui::surface::entry_metadata_layout(reader, values);
    let location_row = metadata.location.expect("location row is laid out");

    // Expected wrapping at the row's real (border-inset) width.
    let wrapped = crate::tui::surface::location_wrapped_lines(
        location_row.prefix_width,
        location_row.rect.width,
        "Grand Central Station Cafe",
    );
    assert!(wrapped.len() >= 2, "label should wrap: {wrapped:?}");
    let continuation = wrapped[1].chars().next().unwrap().to_string();

    let backend = render_app(app, 24, 60);
    let buffer = backend.buffer();

    assert_eq!(location_row.rect.height as usize, wrapped.len());
    // Line 0 leads with the bold "Location: " label; the continuation line runs
    // flush-left (first wrapped word, no prefix, no leading space).
    assert_eq!(
        buffer
            .cell((location_row.rect.x, location_row.rect.y))
            .unwrap()
            .symbol(),
        "L"
    );
    assert_eq!(
        buffer
            .cell((location_row.rect.x, location_row.rect.y + 1))
            .unwrap()
            .symbol(),
        continuation
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
fn reader_wraps_metadata_rows_without_leading_space_or_separator() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
            entry_dir.join("a.md"),
            "+++\nschema_version = 1\ntags = [\"work\", \"personal\", \"health\"]\nfeelings = [\"calm\", \"focused\", \"tired\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Reader;

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
    let reader = Rect::new(0, 0, 24, 60 - expanded_footer_height(&app, 24));
    let values = metadata_values(&tags, &feelings, None);
    let metadata = crate::tui::surface::entry_metadata_layout(reader, values);
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
        metadata_at_point(reader, feelings_row.rect.x, feelings_row.rect.y + 1, values),
        Some((MetadataChip::Feelings, "focused".to_string()))
    );
    assert_eq!(
        metadata_at_point(reader, tags_row.rect.x, tags_row.rect.y + 1, values),
        Some((MetadataChip::Tags, "personal".to_string()))
    );
}

#[test]
fn short_reader_scrolls_metadata_after_body() {
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
                "+++\nschema_version = 1\ntags = [\"tiny-screen\"]\nfeelings = [\"focused\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\n{body}\n",
            ),
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Reader;

    let top = render_text(app, 80, 20);
    assert!(!top.contains("Tags: tiny-screen"));

    let mut app = new_app(Config::new(dir.path().to_path_buf()));
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Reader;
    app.nav.scroll.reader = u16::MAX;

    let bottom = render_text(app, 80, 20);
    assert!(bottom.contains("Feelings: focused"));
    assert!(bottom.contains("Tags: tiny-screen"));
}

#[test]
fn metadata_pins_only_when_body_keeps_min_height() {
    let tags = vec!["x".to_string()];
    let values = EntryMetadataValues {
        tags: &tags,
        people: &[],
        activities: &[],
        feelings: &[],
        mood: None,
        location: None,
    };
    // Separator + one tag row = 2 metadata rows; inner height = area.height - 2. The
    // body needs 20 lines, so pin only once the inner height reaches 22 (area 24).
    assert!(metadata_scrolls_with_body(Rect::new(0, 0, 80, 23), values)); // inner 21 → body 19
    assert!(!metadata_scrolls_with_body(Rect::new(0, 0, 80, 24), values)); // inner 22 → body 20
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
    let area = Rect::new(2, 3, 20, 10);
    theme::set_test_theme(theme::test_flat_theme());
    theme::set_chrome_override(Some(crate::tui::theme::ChromeStyle::Flat));
    let flat_bar = scrollbar_bar_rect(area);
    let flat_content = PanelGeometry::new(area).content;
    assert_eq!(flat_bar, Rect::new(20, 4, 1, 8));
    assert_eq!(flat_content.x + flat_content.width, flat_bar.x - 1);
    assert_eq!(flat_bar.x + flat_bar.width + 1, area.x + area.width);

    theme::set_chrome_override(Some(crate::tui::theme::ChromeStyle::Bordered));
    let bordered_bar = scrollbar_bar_rect(area);
    let bordered_content = PanelGeometry::new(area).content;
    assert_eq!(bordered_bar, Rect::new(21, 4, 1, 8));
    assert_eq!(bordered_content.x, area.x + 2);
    assert_eq!(
        bordered_content.x + bordered_content.width,
        bordered_bar.x - 1
    );
    theme::set_chrome_override(None);
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
    let wide_tags = metadata_dialog_layout(Rect::new(0, 0, 120, 30), 20);
    assert_eq!(wide_tags.area.width, 44);
    assert_eq!(wide_tags.list.height, 14);

    let narrow_tags = metadata_dialog_layout(Rect::new(0, 0, 40, 30), 20);
    assert_eq!(narrow_tags.area.x, 0);
    assert_eq!(narrow_tags.area.width, 40);

    // 15 group headers + 155 feelings = 170 rows; the list caps at its max visible rows.
    let wide_feelings = feelings_dialog_layout(Rect::new(0, 0, 120, 30), 170, &[]);
    assert_eq!(wide_feelings.area.width, 44);
    assert_eq!(wide_feelings.list.height, 16);

    let wide_mood = mood_dialog_layout(Rect::new(0, 0, 120, 30));
    assert_eq!(wide_mood.area.width, 90);

    let narrow_mood = mood_dialog_layout(Rect::new(0, 0, 80, 30));
    assert_eq!(narrow_mood.area.x, 0);
    assert_eq!(narrow_mood.area.width, 80);
}

#[test]
fn feelings_dialog_folds_groups_and_marks_disclosure() {
    use crate::tui::app::EditFeelingState;
    use notema_domain::{Feeling, FeelingGroup};

    static GROUPS: &[FeelingGroup] = &[
        FeelingGroup {
            name: "Peaceful",
            feelings: &[
                Feeling {
                    name: "calm",
                    search_aliases: &[],
                },
                Feeling {
                    name: "content",
                    search_aliases: &[],
                },
            ],
        },
        FeelingGroup {
            name: "Joyful",
            feelings: &[Feeling {
                name: "happy",
                search_aliases: &[],
            }],
        },
    ];
    let mut state = EditFeelingState::new(GROUPS, vec!["calm".into()]);
    state.expanded[1] = true;
    let rows = render_to_rows(60, 24, |frame| {
        dialogs::draw_edit_feelings_dialog(frame, &mut state, crate::tui::state::HoverTarget::None)
    });

    // Collapsed group: header keeps its stored casing (no all-caps), carries a
    // trailing ▸ and a selected count, and its feelings are hidden.
    let collapsed = rows.iter().find(|row| row.contains("Peaceful")).unwrap();
    assert!(
        !collapsed.contains('['),
        "header must not render a checkbox"
    );
    assert!(collapsed.contains('▸'), "collapsed header shows ▸");
    assert!(
        collapsed.contains("(1)"),
        "collapsed header shows selected count"
    );
    // "calm" appears in the selected summary; it must NOT appear as a list row.
    assert!(!rows.iter().any(|row| row.contains("[x] calm")));

    // Expanded group: ▾ marker and its feelings render with checkboxes.
    let expanded = rows.iter().find(|row| row.contains("Joyful")).unwrap();
    assert!(expanded.contains('▾'), "expanded header shows ▾");
    assert!(rows.iter().any(|row| row.contains("[ ] happy")));

    // The selected-feelings summary lists picks from any group.
    assert!(rows.iter().any(|row| row.contains("Selected: calm")));
}

#[test]
fn feelings_dialog_shows_no_matches_when_filter_is_empty() {
    use crate::tui::app::EditFeelingState;
    use notema_domain::{Feeling, FeelingGroup};

    static GROUPS: &[FeelingGroup] = &[FeelingGroup {
        name: "Peaceful",
        feelings: &[Feeling {
            name: "calm",
            search_aliases: &["composed"],
        }],
    }];
    let mut state = EditFeelingState::new(GROUPS, Vec::new());
    // A query matching neither the feeling nor its alias collapses the list.
    state.input = "zzz-nope".into();
    state.rebuild_filter();

    let rows = render_to_rows(60, 24, |frame| {
        dialogs::draw_edit_feelings_dialog(frame, &mut state, crate::tui::state::HoverTarget::None)
    });
    assert!(
        rows.iter().any(|row| row.contains("(no matches)")),
        "an empty filter must still surface the no-matches line"
    );
}

#[test]
fn confirm_delete_shows_message_then_buttons() {
    let rows = render_confirm_delete_rows(80, 20);
    let title_row = rows
        .iter()
        .position(|row| row.contains("Confirm Delete"))
        .unwrap();
    let message_row = rows
        .iter()
        .position(|row| row.contains("Move entry to trash?"))
        .unwrap();
    let button_row = rows
        .iter()
        .position(|row| row.contains("Delete") && row.contains("Cancel"))
        .unwrap();

    // Message sits just below the border/title; the buttons follow, below it.
    assert_eq!(message_row, title_row + 1);
    assert!(button_row > message_row);
}

#[test]
fn edit_tags_dialog_keeps_help_visible_below_spacer() {
    let all_values: Vec<(String, usize)> = (0..20)
        .map(|index| (format!("tag-{index:02}"), index))
        .collect();
    let filtered: Vec<usize> = (0..all_values.len()).collect();
    let active_len = all_values.len();
    let rendered = render_edit_tags_dialog_text(
        EditMetadataState::new(
            MetadataKind::Tags,
            all_values,
            filtered,
            Vec::new(),
            active_len,
        ),
        200,
        20,
    );

    assert!(rendered.contains("[ ] tag-00 (0)"));
    assert!(rendered.contains("space  toggle"));
    assert!(rendered.contains("tab  input"));
    assert!(rendered.contains("enter  save"));
    assert!(rendered.contains("esc  cancel"));
}

#[test]
fn edit_tags_dialog_keeps_list_gutter_when_selection_is_scrolled_out() {
    let all_values: Vec<(String, usize)> = (0..20)
        .map(|index| (format!("tag-{index:02}"), index))
        .collect();
    let filtered: Vec<usize> = (0..all_values.len()).collect();
    let active_len = all_values.len();
    let mut state = EditMetadataState::new(
        MetadataKind::Tags,
        all_values,
        filtered,
        Vec::new(),
        active_len,
    );
    state.list.set_offset(5);

    let rendered = render_edit_tags_dialog_text(state, 200, 20);

    assert!(rendered.contains(" [ ] tag-05 (5)"));
}

#[test]
fn edit_tags_dialog_counts_no_matches_row_when_sizing() {
    let mut state = EditMetadataState::new(
        MetadataKind::Tags,
        vec![("work".to_string(), 1)],
        Vec::new(),
        Vec::new(),
        1,
    );
    state.input = "missing".into();
    state.focus = EditMetadataFocus::Input;
    let rendered = render_edit_tags_dialog_text(state, 200, 12);

    assert!(rendered.contains(" (no matches)"));
    assert!(rendered.contains("enter  add"));
    assert!(rendered.contains("tab  list"));
    assert!(rendered.contains("esc  cancel"));
}

#[test]
fn edit_metadata_input_hint_saves_when_empty_and_adds_when_not_empty() {
    let mut empty =
        EditMetadataState::new(MetadataKind::People, Vec::new(), Vec::new(), Vec::new(), 0);
    empty.focus = EditMetadataFocus::Input;
    let rendered_empty = render_edit_tags_dialog_text(empty, 200, 12);
    assert!(rendered_empty.contains("enter  save"));
    assert!(rendered_empty.contains("tab  list"));
    assert!(rendered_empty.contains("esc  cancel"));

    let mut with_value =
        EditMetadataState::new(MetadataKind::People, Vec::new(), Vec::new(), Vec::new(), 0);
    with_value.focus = EditMetadataFocus::Input;
    with_value.input = "alex".into();
    let rendered_value = render_edit_tags_dialog_text(with_value, 200, 12);
    assert!(rendered_value.contains("enter  add"));
    assert!(rendered_value.contains("tab  list"));
    assert!(rendered_value.contains("esc  cancel"));
}

#[test]
fn entry_hit_testing_ignores_month_divider_and_maps_boxed_entries() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nFirst preview\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nSecond preview\n",
    )
    .unwrap();
    let config = Config::new(dir.path().to_path_buf());
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
            RowMeta {
                item_index: None,
                height: 1,
            },
            RowMeta {
                item_index: Some(0),
                height: 4,
            },
            RowMeta {
                item_index: None,
                height: 1,
            },
            RowMeta {
                item_index: Some(1),
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
            format!("+++\nschema_version = 1\n[datetime]\ncreated_at = \"{ts}\"\n+++\n\n# e{index}\nBody text\n"),
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
    let mut app = new_app(Config::new(dir.path().to_path_buf()));
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
fn reader_renders_feelings_metadata() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
            entry_dir.join("a.md"),
            "+++\nschema_version = 1\nfeelings = [\"calm\", \"focused\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Reader;

    let rendered = render_text(app, 120, 20);

    assert!(rendered.contains("Feelings: calm | focused"));
}

#[test]
fn reader_renders_indented_mermaid_diagram() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
            entry_dir.join("a.md"),
            "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\n```mermaid\n  graph TD\n      A[Open journal] --> B[Write entry]\n      B --> C{Preview}\n      C -->|looks good| D[Save]\n      C -->|needs work| B\n  ```\n",
        )
        .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Reader;

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
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    fs::write(
        work_entry_dir.join("b.md"),
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nBody\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("personal")).unwrap();

    let config = Config::new(root);
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

    let mut reader_focus_app = app_with_entry();
    reader_focus_app.nav.focus = Focus::Reader;
    let reader_focus = render_text(reader_focus_app, 57, 16);
    assert!(!reader_focus.contains(" Entries "));
    assert!(!reader_focus.contains(" Journals "));
    assert!(reader_focus.contains("Body"));
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
fn selected_journal_and_entry_remain_reversed_when_reader_is_focused() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Reader;

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
fn multi_col_fullscreen_takes_the_whole_width_and_hides_columns() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Reader;
    app.nav.reader_fullscreen = true;

    let layout = tui_layout(Rect::new(0, 0, 130, 20), &app);
    assert!(layout.journals.is_none());
    assert!(layout.entries.is_none());
    assert_eq!(layout.reader.unwrap().area.width, 130);

    let text = render_text(app, 130, 20);
    assert!(!text.contains(" Journals "));
    assert!(!text.contains(" Entries "));
    assert!(text.contains("Body"));
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
            .cell((29, 3))
            .unwrap()
            .modifier
            .contains(Modifier::REVERSED)
    );
}

/// An `App` with a `work` journal holding one entry carrying mood, feelings, and
/// a person, with `work` selected and the Journals column focused (so the tabbed
/// insights panel is the visible right pane).
fn app_with_metadata_entry() -> App {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\ntags = [\"work\"]\nfeelings = [\"calm\"]\npeople = [\"alex\"]\nactivities = [\"running\"]\nmood = 3\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    std::mem::forget(dir);
    app
}

/// Put `app` into the state where the insights panel is the visible, focused
/// right pane: browsing with the panel focused and no entry selected.
fn focus_insights(app: &mut App, tab: insights::InsightsTab) {
    app.nav.selected_entry_index = None;
    app.nav.focus = Focus::Insights;
    app.nav.insights_tab = tab;
}

#[test]
fn insights_panel_shows_all_tabs_in_its_border() {
    let mut app = app_with_entry();
    focus_insights(&mut app, insights::InsightsTab::Overview);
    // Wide enough that the strip uses full titles rather than short/initials.
    let text = render_text(app, 170, 20);

    for title in ["Overview", "Writing", "Feelings", "Drivers"] {
        assert!(text.contains(title), "tab bar missing {title}: {text}");
    }
}

#[test]
fn insights_overview_tab_shows_journal_summary() {
    let mut app = app_with_entry();
    focus_insights(&mut app, insights::InsightsTab::Overview);
    let text = render_text(app, 140, 20);

    // The paired cards plus the totals in the title box.
    assert!(text.contains("Lifts you"));
    assert!(text.contains("Drains you"));
    assert!(text.contains("Happiest day"));
    assert!(text.contains("Active days"));
    assert!(text.contains("entry") || text.contains("entries"));
}

#[test]
fn insights_switching_tab_changes_the_body() {
    let mut app = app_with_entry();
    focus_insights(&mut app, insights::InsightsTab::Feelings);

    let text = render_text(app, 140, 20);

    // The lone fixture entry has no mood and no feelings, so the merged Feelings tab
    // shows its empty state rather than the Overview cards.
    assert!(text.contains("No mood or feelings logged yet"));
    assert!(!text.contains("Days"));
}

#[test]
fn insights_feelings_tab_renders_frequency_bar() {
    let mut app = app_with_metadata_entry();
    focus_insights(&mut app, insights::InsightsTab::Feelings);

    // Tall enough that the feelings table still fits below Balance + the breakdowns.
    let text = render_text(app, 140, 26);

    assert!(text.contains("calm"));
    assert!(text.contains('▓'), "expected a bar glyph: {text}");
}

/// A journal whose entries make `alex` a clear mood lift and `rain` a clear
/// drain, each appearing enough times (≥3) to clear the Drivers noise guard.
fn app_with_drivers() -> App {
    let dir = tempdir().unwrap();
    let base = dir.path().join("work");
    let specs = [
        (5, "people = [\"alex\"]"),
        (5, "people = [\"alex\"]"),
        (5, "people = [\"alex\"]"),
        (-5, "tags = [\"rain\"]"),
        (-5, "tags = [\"rain\"]"),
        (-5, "tags = [\"rain\"]"),
    ];
    for (index, (mood, meta)) in specs.iter().enumerate() {
        let day = index + 1;
        let entry_dir = base.join(format!("2026-07-{day:02}"));
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(
            entry_dir.join("a.md"),
            format!(
                "+++\nschema_version = 1\n{meta}\nmood = {mood}\n[datetime]\ncreated_at = \"2026-07-{day:02}T10:00:00+02:00\"\n+++\n\n# E\nBody\n"
            ),
        )
        .unwrap();
    }
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    std::mem::forget(dir);
    app
}

#[test]
fn insights_drivers_tab_ranks_lifts_and_drains() {
    let mut app = app_with_drivers();
    focus_insights(&mut app, insights::InsightsTab::Drivers);

    let text = render_text(app, 140, 20);

    // People, activities, and tags are merged into one ranking.
    assert!(text.contains("alex"), "lifting person missing: {text}");
    assert!(text.contains("rain"), "draining tag missing: {text}");
}

#[test]
fn insights_drivers_tab_renders_headed_table_with_mood_bar() {
    let mut app = app_with_drivers();
    focus_insights(&mut app, insights::InsightsTab::Drivers);
    // Expanded to full screen — the "bigger screen" case with room for the bar.
    app.nav.insights_fullscreen = true;

    let text = render_text(app, 140, 20);

    assert!(text.contains("Count"), "table header missing: {text}");
    assert!(
        text.contains("Drains / lifts"),
        "bar column missing: {text}"
    );
    assert!(text.contains('│'), "bar centre marker missing: {text}");
}

/// An app whose sole entry lists `count` people (`p00`..), so the People tab has a
/// list long enough to scroll.
/// A journal where people `p00`..`p{count-1}` each ride high moods across three
/// entries (clearing the ≥3 noise guard), plus baseline low-mood entries — so the
/// Drivers ranking is a list long enough to scroll.
fn app_with_many_drivers(count: usize) -> App {
    let dir = tempdir().unwrap();
    let base = dir.path().join("work");
    let people: Vec<String> = (0..count).map(|i| format!("\"p{i:02}\"")).collect();
    let people = people.join(", ");
    // Three high-mood entries listing everyone, then two low-mood baselines.
    let specs = [
        (5, format!("people = [{people}]")),
        (5, format!("people = [{people}]")),
        (5, format!("people = [{people}]")),
        (-3, String::new()),
        (-3, String::new()),
    ];
    for (index, (mood, meta)) in specs.iter().enumerate() {
        let day = index + 1;
        let entry_dir = base.join(format!("2026-07-{day:02}"));
        fs::create_dir_all(&entry_dir).unwrap();
        let meta = if meta.is_empty() {
            String::new()
        } else {
            format!("{meta}\n")
        };
        fs::write(
            entry_dir.join("a.md"),
            format!(
                "+++\nschema_version = 1\n{meta}mood = {mood}\n[datetime]\ncreated_at = \"2026-07-{day:02}T10:00:00+02:00\"\n+++\n\n# A\nBody\n"
            ),
        )
        .unwrap();
    }
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    std::mem::forget(dir);
    app
}

#[test]
fn insights_list_scrolls_to_reveal_later_rows() {
    let focused_drivers = |scroll: u16| {
        let mut app = app_with_many_drivers(30);
        focus_insights(&mut app, insights::InsightsTab::Drivers);
        app.nav.insights_fullscreen = true;
        app.nav.scroll.insights = scroll;
        render_text(app, 120, 12)
    };

    // A short panel can't show every driver: the first is visible, a late one isn't.
    let top = focused_drivers(0);
    assert!(top.contains("p00"), "first row should be visible: {top}");
    assert!(!top.contains("p29"), "last row should be off-screen: {top}");

    // Jumping to the end (like the End key) reveals the last row and drops the first;
    // the render clamps the saturated offset to the final page.
    let bottom = focused_drivers(u16::MAX);
    assert!(
        bottom.contains("p29"),
        "last row should scroll into view: {bottom}"
    );
    assert!(
        !bottom.contains("p00"),
        "first row should scroll away: {bottom}"
    );
}

#[test]
fn insights_feelings_tab_shows_balance_and_feeling_table() {
    let mut app = app_with_metadata_entry();
    focus_insights(&mut app, insights::InsightsTab::Feelings);

    let text = render_text(app, 140, 30);

    assert!(
        text.contains("Balance"),
        "feelings tab missing balance: {text}"
    );
    assert!(
        text.contains("calm"),
        "feelings table missing the feeling: {text}"
    );
}

#[test]
fn insights_writing_tab_renders_habit_sections() {
    let mut app = app_with_metadata_entry();
    focus_insights(&mut app, insights::InsightsTab::Writing);
    // Wide + full screen so the weekday/hour charts sit side by side.
    app.nav.insights_fullscreen = true;

    let text = render_text(app, 140, 20);

    assert!(
        text.contains("Streak"),
        "writing tab missing streak: {text}"
    );
    assert!(
        text.contains("By weekday") && text.contains("By hour"),
        "writing tab missing side-by-side histograms: {text}"
    );
}

#[test]
fn insights_feelings_tab_renders_mood_breakdowns() {
    let mut app = app_with_metadata_entry();
    focus_insights(&mut app, insights::InsightsTab::Feelings);
    // Wide + full screen so the three breakdown charts sit side by side.
    app.nav.insights_fullscreen = true;

    let text = render_text(app, 140, 24);

    // The merged tab carries the signed mood breakdowns below Balance...
    assert!(
        text.contains("By year") && text.contains("By weekday") && text.contains("By month"),
        "feelings tab missing the mood breakdown charts: {text}"
    );
    // ...and drops the old Mood tab's abstract series.
    assert!(
        !text.contains("Mood over time"),
        "mood-over-time series should be gone: {text}"
    );
}

#[test]
fn insights_tab_hit_test_maps_border_columns_to_tabs() {
    // Inner width 47 fits all four full labels: " Overview · Writing · Mood /
    // Feelings · Drivers", the title starting one past the corner at 75.
    let area = Rect::new(74, 0, 49, 19);
    assert_eq!(
        insights_tab_at(area, 78, 0),
        Some(insights::InsightsTab::Overview) // 76..84
    );
    assert_eq!(
        insights_tab_at(area, 90, 0),
        Some(insights::InsightsTab::Writing)
    ); // 87..94
    assert_eq!(
        insights_tab_at(area, 100, 0),
        Some(insights::InsightsTab::Feelings) // 97..112
    );
    assert_eq!(
        insights_tab_at(area, 118, 0),
        Some(insights::InsightsTab::Drivers)
    ); // 115..122
    // The corner, the gaps, and other rows are not tabs.
    assert_eq!(insights_tab_at(area, 74, 0), None);
    assert_eq!(insights_tab_at(area, 85, 0), None); // " · " between Overview and Writing
    assert_eq!(insights_tab_at(area, 96, 0), None); // " · " between Writing and Mood / Feelings
    assert_eq!(insights_tab_at(area, 78, 1), None);
}

#[test]
fn insights_active_tab_inverts_only_when_panel_is_focused() {
    // Focused: the active tab in the border uses the reversed style.
    let mut focused = app_with_entry();
    focus_insights(&mut focused, insights::InsightsTab::Overview);
    let backend = render_app(focused, 140, 20);
    assert!(
        insights_border_has_reversed_text(&backend, 140),
        "focused panel should invert its active tab"
    );

    // Unfocused (Journals): the active tab is bold, not reversed. A focused
    // Journals panel reverses its own title, so the check is scoped to the insights
    // column to ignore that.
    let mut unfocused = app_with_entry();
    unfocused.nav.selected_entry_index = None;
    unfocused.nav.focus = Focus::Journals;
    let backend = render_app(unfocused, 140, 20);
    assert!(
        !insights_border_has_reversed_text(&backend, 140),
        "unfocused panel must not invert its active tab"
    );
}

/// Whether any non-blank cell in the insights panel's top border row is reversed —
/// the mark of the focused active tab. Scoped to the right-hand insights column
/// (past the journal + entry columns) so a focused Journals title doesn't count.
fn insights_border_has_reversed_text(backend: &TestBackend, width: u16) -> bool {
    const STATS_COLUMN_X: usize = 74;
    backend
        .buffer()
        .content()
        .iter()
        .take(width as usize)
        .skip(STATS_COLUMN_X)
        .any(|cell| cell.symbol() != " " && cell.modifier.contains(Modifier::REVERSED))
}

#[test]
fn journal_footer_omits_entry_actions() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Journals;

    let text = footer_text(&app, 200);

    assert!(!text.contains("enter  view"));
    assert!(!text.contains("e  edit"));
    assert!(!text.contains("d  del"));
}

#[test]
fn entries_footer_includes_entry_actions_when_an_entry_is_selected() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let text = footer_text(&app, 200);

    assert!(text.contains("enter  view"));
    assert!(text.contains("e  edit"));
    assert!(text.contains("d  del"));
}

#[test]
fn expanded_entry_footer_includes_inline_entry_actions() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Reader;

    let inline_text = footer_text(&app, 200);
    let expanded_text = expanded_footer_text(&app, 200);

    for label in [
        "n  new entry",
        "e  edit",
        "d  del",
        "ctrl+g  metadata",
        "/  search",
        "q  quit",
    ] {
        assert!(inline_text.contains(label));
        assert!(expanded_text.contains(label));
    }
    // The per-field metadata shortcuts are folded into the metadata popup, so no
    // longer appear as their own footer chips in either form.
    for label in [
        "t  tags",
        "p  ppl",
        "a  act",
        "f  feel",
        "m  mood",
        "l  location",
    ] {
        assert!(!inline_text.contains(label));
        assert!(!expanded_text.contains(label));
    }
    // Single-column full screen (the flag is unset): Left also exits, so it is
    // listed alongside Enter/Esc.
    assert!(expanded_text.contains("enter/esc/←  close"));

    // Multi-column full screen: Left is inert (Esc collapses), so it drops from the
    // close hint.
    app.nav.reader_fullscreen = true;
    assert!(expanded_footer_text(&app, 200).contains("enter/esc  close"));
}

#[test]
fn expanded_entry_draws_confirm_delete_overlay() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Reader;
    app.begin_confirm_delete();

    let text = render_text(app, 80, 20);

    assert!(text.contains("Confirm Delete"));
    assert!(text.contains("Move entry to trash?"));
    assert!(text.contains("Delete") && text.contains("Cancel"));
}

#[test]
fn entries_footer_omits_entry_actions_without_a_selection() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("work")).unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;

    let text = footer_text(&app, 200);

    assert!(!text.contains("enter  view"));
    assert!(!text.contains("e  edit"));
    assert!(!text.contains("d  del"));
}

#[test]
fn search_results_footer_shows_escape_and_entry_actions() {
    let mut app = app_with_entry();
    app.nav.mode = Mode::Search;
    app.nav.focus = Focus::Entries;
    app.search.query = "body".into();
    app.search.hits = vec![SearchHit {
        id: app.library.entries[0].id.clone(),
        journal: "work".to_string(),
        created_at: None,
        title: "A".to_string(),
        preview: "Body".to_string(),
        starred: false,
    }];

    let text = footer_text(&app, 200);

    // The query now lives on the entry panel's top-right border, not the footer.
    assert!(!text.contains("Search all: body"));
    assert!(text.contains("enter  view"));
    assert!(text.contains("esc  exit search"));
    assert!(!text.contains("type query"));
    assert!(!text.contains("backspace"));
    assert!(!text.contains("e  edit"));
    assert!(!text.contains("d  del"));
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

    let width = 60;
    let origin_y = 18;
    let text = footer_text(&app, width);
    let (row_index, line) = text
        .split('\n')
        .enumerate()
        .find(|(_, line)| line.contains("ctrl+g  metadata"))
        .expect("metadata hint present");
    let col = line.find("ctrl+g  metadata").unwrap() as u16;

    assert_eq!(
        footer_hint_id_at_point(&app, 0, origin_y, width, col, origin_y + row_index as u16),
        Some(HintId::OpenMetadataMenu)
    );
}

#[test]
fn footer_hint_routing_uses_typed_ids() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;
    let text = footer_text(&app, 200);

    assert_eq!(
        footer_hint_id_at(&app, 0, 200, text.find("ctrl+g  metadata").unwrap() as u16),
        Some(HintId::OpenMetadataMenu)
    );
    assert_eq!(
        footer_hint_id_at(&app, 0, 200, text.find("e  edit").unwrap() as u16),
        Some(HintId::EditSelected)
    );
}

#[test]
fn expanded_footer_hint_routing_uses_typed_ids() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Reader;
    let width = 120;
    let origin_y = 19;
    let text = expanded_footer_text(&app, width);
    let (row_index, line) = text
        .split('\n')
        .enumerate()
        .find(|(_, line)| line.contains("ctrl+g  metadata"))
        .expect("metadata hint present");
    let col = line.find("ctrl+g  metadata").unwrap() as u16;

    assert_eq!(
        expanded_footer_hint_id_at_point(
            &app,
            0,
            origin_y,
            width,
            1 + col,
            origin_y + row_index as u16
        ),
        Some(HintId::OpenMetadataMenu)
    );
}

/// Every hint is clickable at its own rendered position, whatever row the grid
/// places it on.
fn assert_hints_routable(hints: &[Hint], width: u16) {
    let text = hint_grid_text(hints, width);
    for (row_index, line) in text.split('\n').enumerate() {
        for hint in hints {
            // The `key  label` pair is unambiguous within a row (a bare label can
            // be a substring of another hint's text).
            let needle = format!("{}  {}", hint.key_hint, hint.label);
            if let Some(col) = line.find(&needle) {
                assert_eq!(
                    hint_id_at_wrapped(hints, 0, 0, width, col as u16, row_index as u16),
                    Some(hint.id),
                    "hint {:?} on row {row_index}",
                    hint.label
                );
            }
        }
    }
}

#[test]
fn dialog_hints_wrap_and_remain_clickable_by_row() {
    let hints = metadata_dialog_hints(EditMetadataFocus::List, true);

    assert!(hint_height(hints, 29) >= 2, "expected the hints to wrap");
    assert_hints_routable(hints, 29);
}

#[test]
fn dialog_hint_routing_uses_typed_ids() {
    assert_hints_routable(metadata_dialog_hints(EditMetadataFocus::List, true), 200);
    assert_hints_routable(metadata_dialog_hints(EditMetadataFocus::Input, true), 200);
    assert_hints_routable(metadata_dialog_hints(EditMetadataFocus::Input, false), 200);
    assert_hints_routable(feelings_dialog_hints(EditMetadataFocus::List), 200);
    assert_hints_routable(mood_dialog_hints(), 200);
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
        created_at: created_at.map(notema_domain::Timestamp::parse),
        edited_at: None,
        preview: preview.to_string(),
        activities: Vec::new(),
        feelings: Vec::new(),
        people: Vec::new(),
        tags: Vec::new(),
        mood: None,
        starred: false,
        location: None,
        weather: None,
        celestial: None,
        air_quality: None,
        import: None,
        body: String::new(),
        word_count: 0,
        search_haystack: String::new(),
        warning: None,
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
    let rendered = rendered_lines(&entry_box_lines(None, "", "just a preview", None, None, 30));

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
        None,
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
        created_at: Some(notema_domain::Timestamp::parse("2026-07-01T10:23:00+02:00")),
        edited_at: None,
        preview: String::new(),
        activities: Vec::new(),
        feelings: Vec::new(),
        people: Vec::new(),
        tags: Vec::new(),
        mood: None,
        starred: false,
        location: None,
        weather: None,
        celestial: None,
        air_quality: None,
        import: None,
        body: String::new(),
        word_count: 0,
        search_haystack: String::new(),
        warning: None,
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
        edited_at: None,
        preview: String::new(),
        activities: Vec::new(),
        feelings: Vec::new(),
        people: Vec::new(),
        tags: Vec::new(),
        mood: None,
        starred: false,
        location: None,
        weather: None,
        celestial: None,
        air_quality: None,
        import: None,
        body: String::new(),
        word_count: 0,
        search_haystack: String::new(),
        warning: None,
    };

    assert_eq!(entry_month_label(&entry), Some("July 2026".to_string()));
    assert_eq!(entry_day_label(&entry), Some("Wednesday 01".to_string()));
}

fn render_unlock_text(input: &str, error: Option<&str>) -> String {
    let mut field = crate::tui::text_input::PassphraseInput::default();
    for ch in input.chars() {
        field.insert(ch);
    }
    render_to_text(60, 16, |frame| {
        draw_unlock(frame, &field, error);
    })
}

#[test]
fn unlock_screen_masks_passphrase_and_draws_border() {
    let text = render_unlock_text("hunter2", None);
    // Bordered fullscreen chrome with the title and hint.
    assert!(text.contains("Unlock Notema"));
    assert!(text.contains("enter unlock"));
    assert!(text.contains("esc quit"));
    // The field sits in its own bordered box titled on the top-left.
    assert!(text.contains("Enter Password"));
    // The raw passphrase is never echoed; one '*' per character is.
    assert!(!text.contains("hunter2"));
    assert!(text.contains("*******"));
    // A standing hint sits below the field when there's no error.
    assert!(text.contains("Enter your passphrase to unlock"));
}

#[test]
fn unlock_screen_replaces_hint_with_error() {
    let text = render_unlock_text("", Some("Incorrect passphrase"));
    // The error takes the hint's place after a wrong passphrase.
    assert!(text.contains("Incorrect passphrase"));
    assert!(!text.contains("Enter your passphrase to unlock"));
}

fn render_unlock_rows(width: u16, height: u16, error: Option<&str>) -> Vec<String> {
    let input = crate::tui::text_input::PassphraseInput::default();
    render_to_rows(width, height, |frame| {
        draw_unlock(frame, &input, error);
    })
    .into_iter()
    .map(|row| row.trim().to_string())
    .collect()
}

#[test]
fn unlock_status_wraps_on_a_narrow_terminal() {
    // Too narrow to fit the hint on one line: it must wrap across rows rather
    // than clip, so every word survives.
    let rows = render_unlock_rows(24, 20, None);
    let your_row = rows.iter().position(|r| r.contains("your"));
    let phrase_row = rows.iter().position(|r| r.contains("passphrase"));
    // Both hint words render in full (not truncated) on separate rows.
    assert!(your_row.is_some() && phrase_row.is_some());
    assert_ne!(your_row, phrase_row);
}

fn render_pending_notice_text(device_name: &str, notice: &AccessNotice) -> String {
    render_to_text(72, 20, |frame| {
        draw_pending_notice(frame, device_name, notice)
    })
}

#[test]
fn pending_notice_wraps_in_the_journal_chrome_frame() {
    let text =
        render_pending_notice_text("phone", &AccessNotice::NeedsEnroll { retired_key: false });
    // Outer Notema chrome frame with its dismiss hint, plus the inner state box.
    assert!(text.contains("Notema"));
    assert!(text.contains("any key to exit"));
    assert!(text.contains("Not authorized"));
    assert!(text.contains("Device 'phone'"));
    assert!(text.contains(crate::ENROLL_CMD));
}

#[test]
fn pending_notice_only_mentions_a_retired_key_when_one_was_retired() {
    let retired =
        render_pending_notice_text("phone", &AccessNotice::NeedsEnroll { retired_key: true });
    assert!(retired.contains("old key has been retired"));

    // A never-enrolled device never had a key, so the line is omitted.
    let fresh = render_pending_notice_text("", &AccessNotice::NeedsEnroll { retired_key: false });
    assert!(!fresh.contains("old key has been retired"));
    // A keyless device reads as the sentence subject "This device", not a name.
    assert!(fresh.contains("This device"));
}

#[test]
fn pending_notice_awaiting_points_at_approval() {
    let text = render_pending_notice_text("phone", &AccessNotice::AwaitingApproval);
    assert!(text.contains("Awaiting approval"));
    assert!(text.contains(&format!("{} phone", crate::APPROVE_CMD)));
    assert!(!text.contains("old key has been retired"));
}

#[test]
fn disable_notice_renders_in_the_journal_chrome_frame() {
    let backend = TestBackend::new(72, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(draw_disable_notice).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Notema"));
    assert!(text.contains("any key to continue"));
    assert!(text.contains("Encryption disabled"));
    assert!(text.contains("notema encryption enable"));
}

#[test]
fn internal_editor_renders_in_reader_pane() {
    let mut app = app_with_entry();
    app.open_editor_for_selected();
    let text = render_text(app, INLINE_READER_MIN_WIDTH, 30);
    // The textarea shows the raw markdown source (with the leading `#`), unlike
    // the viewer which renders the heading, so the literal `# A` proves the
    // editor drew in the pane.
    assert!(text.contains("# A"));
    // The editor footer replaces the browse hints.
    assert!(text.contains("ctrl+s"));
}

#[test]
fn internal_editor_renders_full_screen() {
    let mut app = app_with_entry();
    app.open_editor_for_selected();
    app.nav.reader_fullscreen = true;
    let text = render_text(app, INLINE_READER_MIN_WIDTH, 30);
    assert!(text.contains("# A"));
    assert!(text.contains("ctrl+s"));
}

#[test]
fn internal_editor_new_entry_renders_in_pane_not_insights() {
    let mut app = app_with_journals(&["work"]);
    app.select_journal_by_name("work");
    app.open_editor_for_new();
    // Not fullscreen: the entry list column is still present alongside the editor.
    let text = render_text(app, INLINE_READER_MIN_WIDTH, 30);
    assert!(text.contains("New entry")); // editor pane title, not the insights panel
    assert!(text.contains("ctrl+s")); // editor footer
}

#[test]
fn internal_editor_metadata_menu_renders() {
    let mut app = app_with_entry();
    app.open_editor_for_selected();
    app.editor.as_mut().unwrap().prompt = crate::tui::editor_state::EditorPrompt::MetadataMenu;
    let text = render_text(app, INLINE_READER_MIN_WIDTH, 30);
    assert!(text.contains("Add Metadata"));
    assert!(text.contains("Feelings"));
}

/// The editor's metadata section renders the entry's location just like the
/// viewer — both go through `EntryMetadata::from_metadata`, so a front-matter
/// field can't show in one mode and vanish in the other.
#[test]
fn internal_editor_shows_entry_location() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n\n[location]\nname = \"Testville Cafe\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    let mut app = new_app(Config::new(dir.path().to_path_buf()));
    app.select_journal_by_name("work");
    app.open_editor_for_selected();

    let text = render_text(app, INLINE_READER_MIN_WIDTH, 30);
    assert!(text.contains("Testville Cafe"), "editor pane was:\n{text}");
}

/// The shortcut overlay shows the full bordered grid when tall enough and
/// collapses to chrome-less rows (no box-drawing) when it is not.
#[test]
fn editor_shortcuts_collapses_when_short() {
    let has_grid = |h: u16| {
        render_to_text(64, h, |frame| {
            super::menus::draw_editor_shortcuts(frame, &mut 0)
        })
        .contains('┼')
    };
    assert!(has_grid(44), "tall terminal shows the bordered grid");
    assert!(!has_grid(20), "short terminal collapses to plain rows");
}

#[test]
fn editor_shortcuts_hit_test_action_rows() {
    let area = Rect::new(0, 0, 64, 44);
    let mut found = Vec::new();
    let mut close_found = false;
    for y in 0..area.height {
        for x in 0..area.width {
            if let Some(id) = super::menus::editor_shortcut_hint_at_point(area, 0, x, y) {
                found.push(id);
            }
            close_found |= super::menus::editor_shortcut_close_at_point(area, 0, x, y);
        }
    }

    assert!(found.contains(&HintId::EditorSave));
    assert!(found.contains(&HintId::EditorFullscreen));
    assert!(found.contains(&HintId::EditorMetadata));
    assert!(found.contains(&HintId::EditorDiscard));
    assert!(close_found);
}

#[test]
fn editor_metadata_menu_hit_tests_rows() {
    let area = Rect::new(0, 0, 64, 30);
    let mut found_tags = false;
    let mut found_feelings = false;
    let mut found_mood = false;
    for y in 0..area.height {
        for x in 0..area.width {
            let mode = super::menus::MetadataMenuMode::Editor;
            match super::menus::metadata_menu_choice_at_point(area, mode, x, y) {
                Some(super::menus::MetadataChoice::Metadata(MetadataKind::Tags)) => {
                    found_tags = true;
                }
                Some(super::menus::MetadataChoice::Feelings) => found_feelings = true,
                Some(super::menus::MetadataChoice::Mood) => found_mood = true,
                _ => {}
            }
        }
    }

    assert!(found_tags);
    assert!(found_feelings);
    assert!(found_mood);
}

// ── Settings menu / theme picker ─────────────────────────────────────────────

#[test]
fn settings_menu_lists_the_theme_row_and_hit_tests_it() {
    let text = render_to_text(64, 20, |frame| menus::draw_settings_menu(frame, None));
    assert!(text.contains("Settings"));
    assert!(text.contains("Theme…"));
    assert!(text.contains("enter select · esc close"));

    let area = Rect::new(0, 0, 64, 20);
    let mut found_theme = false;
    let mut close_found = false;
    for y in 0..area.height {
        for x in 0..area.width {
            if matches!(
                menus::settings_menu_choice_at_point(area, x, y),
                Some(menus::SettingsChoice::Theme)
            ) {
                found_theme = true;
            }
            close_found |= menus::settings_menu_close_at_point(area, x, y);
        }
    }
    assert!(found_theme);
    assert!(close_found);
}

#[test]
fn theme_picker_lists_bundled_themes_with_the_active_row_marked() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();

    let rows = render_to_rows(90, 30, |frame| draw(frame, &mut app));
    let text = rows.join("\n");

    // Dialog frame with its title and hint row.
    assert!(text.contains(" Theme "), "dialog title missing:\n{text}");
    assert!(text.contains("enter  apply"));
    assert!(text.contains("esc  revert"));
    // Every bundled theme is listed; the configured one carries the ● marker.
    for name in ["blossom", "classic", "eclipse", "fjord", "grove", "journal"] {
        assert!(text.contains(name), "theme '{name}' missing:\n{text}");
    }
    assert!(text.contains("● blossom"), "active marker missing:\n{text}");
}

#[test]
fn theme_picker_renders_broken_rows_in_the_error_style() {
    let mut app = app_with_journals(&["work"]);
    let themes = crate::tui::theme::themes_dir(&app.config_path);
    fs::create_dir_all(&themes).unwrap();
    fs::write(themes.join("busted.toml"), "surfaces = 12\n").unwrap();
    app.open_theme_picker();
    let state = app.theme_picker_state().unwrap();
    let (len, mode_switchable) = (state.entries.len(), state.mode_switchable());

    let backend = render_app(app, 90, 30);
    let buffer = backend.buffer();
    let rows: Vec<String> = buffer
        .content()
        .chunks(90)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let (y, line) = rows
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains("busted (broken)"))
        .expect("broken row rendered");
    let x = line.find("busted").unwrap() as u16;

    let error_fg = crate::tui::theme::theme().error().fg.unwrap();
    assert_eq!(buffer[(x, y as u16)].fg, error_fg);
    // The layout the mouse handler uses matches where the list was drawn.
    let layout = theme_picker_layout(Rect::new(0, 0, 90, 30), len, mode_switchable);
    assert!(point_in_rect(layout.list, x, y as u16));
}

// ── Flat chrome (bg-layered themes) ──────────────────────────────────────────

mod flat_chrome_tests {
    use super::*;
    use crate::tui::state::{HoverTarget, MetadataKind};
    use crate::tui::theme;

    fn pin_flat() {
        theme::set_test_theme(theme::test_flat_theme());
        theme::set_chrome_override(None);
    }

    fn tags_state() -> EditMetadataState {
        EditMetadataState::new(
            MetadataKind::Tags,
            vec![("work".to_string(), 3), ("home".to_string(), 1)],
            vec![0, 1],
            Vec::new(),
            2,
        )
    }

    fn many_tags_state() -> EditMetadataState {
        let all_values: Vec<_> = (0..20)
            .map(|index| (format!("tag-{index:02}"), index))
            .collect();
        let filtered = (0..all_values.len()).collect();
        EditMetadataState::new(MetadataKind::Tags, all_values, filtered, Vec::new(), 20)
    }

    #[test]
    fn dialogs_drop_borders_for_a_title_row_with_esc_hint() {
        pin_flat();
        let rendered = render_edit_tags_dialog_text(tags_state(), 80, 24);
        assert!(!rendered.contains('┌'), "flat dialog still draws corners");
        assert!(
            !rendered.contains('│'),
            "flat dialog still draws side borders"
        );
        assert!(rendered.contains("Edit Tags"));
        assert!(rendered.contains("esc"));
    }

    #[test]
    fn dialog_surface_carries_the_dialog_background() {
        pin_flat();
        let dialog_bg = theme::test_flat_theme().dialog_bg();
        let backend = render_backend(80, 24, |frame| {
            dialogs::draw_edit_metadata_dialog(
                frame,
                &mut tags_state(),
                crate::tui::state::HoverTarget::None,
            )
        });
        let area = metadata_dialog_layout(Rect::new(0, 0, 80, 24), 2).area;
        let cell = &backend.buffer()[(area.x + 1, area.y + 1)];
        assert_eq!(cell.bg, dialog_bg);
    }

    #[test]
    fn bordered_dialog_list_keeps_one_cell_before_the_frame_and_scrollbar() {
        pin_flat();
        theme::set_chrome_override(Some(crate::tui::theme::ChromeStyle::Bordered));
        let frame_area = Rect::new(0, 0, 80, 20);
        let layout = metadata_dialog_layout(frame_area, 20);
        let backend = render_backend(frame_area.width, frame_area.height, |frame| {
            dialogs::draw_edit_metadata_dialog(frame, &mut many_tags_state(), HoverTarget::None)
        });
        let bar = scrollbar_bar_rect(layout.area);
        let row = layout.list.y;

        assert_eq!(layout.list.x, layout.area.x + 2);
        assert_eq!(layout.list.x + layout.list.width, bar.x - 1);
        assert_eq!(backend.buffer()[(layout.area.x + 1, row)].symbol(), " ");
        assert_eq!(backend.buffer()[(bar.x - 1, row)].symbol(), " ");
        assert_ne!(backend.buffer()[(bar.x, row)].symbol(), " ");
        theme::set_chrome_override(None);
    }

    #[test]
    fn selection_is_a_bg_fill_not_reversed() {
        pin_flat();
        let selection = theme::test_flat_theme().selection();
        let layout = metadata_dialog_layout(Rect::new(0, 0, 80, 24), 2);
        let backend = render_backend(80, 24, |frame| {
            dialogs::draw_edit_metadata_dialog(
                frame,
                &mut tags_state(),
                crate::tui::state::HoverTarget::None,
            )
        });
        let row = layout.list.y;
        let cell = &backend.buffer()[(layout.list.x + 3, row)];
        assert_eq!(cell.bg, selection.bg.unwrap());
        assert!(!cell.modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn focused_panel_gets_a_stripe_instead_of_a_thick_border() {
        pin_flat();
        let app = app_with_journals(&["alpha", "beta"]);
        let backend = render_app(app, 120, 30);
        let rendered: String = backend
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains('┃'), "focus stripe missing");
        // Only the panel borders (thick when focused) must be gone.
        assert!(
            !rendered.contains('┏'),
            "thick border corner leaked into flat chrome"
        );
    }

    #[test]
    fn journal_cards_keep_a_uniform_row_geometry() {
        pin_flat();
        let app = app_with_journals(&["work", "zeta", "old.archived"]);
        let rows = crate::tui::entry_rows::journal_list_rows(&app, 16);
        let meta = crate::tui::entry_rows::rows_meta(&rows);
        // Same shape as the bordered column, one separator row taller: uniform
        // rows (divider included) so scroll and hit-testing stay a multiply.
        let indices: Vec<Option<usize>> = meta.iter().map(|m| m.item_index).collect();
        assert_eq!(indices, vec![Some(0), Some(1), None, Some(2)]);
        assert!(
            meta.iter()
                .all(|m| m.height == crate::tui::render::journal_row_height())
        );
    }

    #[test]
    fn journal_cards_carry_selection_and_element_backgrounds() {
        pin_flat();
        let theme = theme::test_flat_theme();
        let app = app_with_journals(&["work", "zeta"]);
        let layout = tui_layout(Rect::new(0, 0, 120, 30), &app);
        let journals = layout.journals.unwrap();
        let list = journal_list_rect(journals.content);
        let buffer_backend = render_app(app, 120, 30);
        let buffer = buffer_backend.buffer();

        // Cards fill three rows (padding, name, padding); the fourth is the gap.
        let selection_bg = theme.selection().bg.unwrap();
        for y in [list.y, list.y + 1, list.y + 2] {
            assert_eq!(
                buffer[(list.x + 2, y)].bg,
                selection_bg,
                "selected card row {y} misses the selection background"
            );
        }
        let gap = &buffer[(list.x + 2, list.y + 3)];
        assert_ne!(gap.bg, selection_bg, "separator row painted like the card");
        let unselected = &buffer[(list.x + 2, list.y + 5)];
        assert_eq!(unselected.bg, theme.element_bg());

        // No box-drawing left in the journal column.
        for y in journals.content.y..journals.content.bottom() {
            for x in journals.content.x..journals.content.right() {
                let symbol = buffer[(x, y)].symbol();
                assert!(
                    !"┌┐└┘".contains(symbol),
                    "box corner {symbol:?} at ({x},{y}) in flat journal column"
                );
            }
        }
    }

    #[test]
    fn all_journals_search_floods_every_card_but_not_the_divider() {
        pin_flat();
        let theme = theme::test_flat_theme();
        let mut app = app_with_journals(&["work", "zeta", "old.archived"]);
        app.nav.mode = crate::tui::app::Mode::Search;
        app.search.scope = crate::tui::app::SearchScope::AllJournals;

        // ≥ INLINE_READER_MIN_WIDTH so the journal column stays visible in
        // search mode.
        let layout = tui_layout(Rect::new(0, 0, 140, 30), &app);
        let journals = layout.journals.unwrap();
        let list = journal_list_rect(journals.content);
        let backend = render_app(app, 140, 30);
        let buffer = backend.buffer();

        let selection_bg = theme.selection().bg.unwrap();
        // Card name rows at 1, 5 (active) and 13 (archived, after the divider
        // block at rows 8..12).
        for card_y in [list.y + 1, list.y + 5, list.y + 13] {
            assert_eq!(
                buffer[(list.x + 2, card_y)].bg,
                selection_bg,
                "card at row {card_y} not flooded by the all-journals search"
            );
        }
        let divider = &buffer[(list.x + 2, list.y + 9)];
        assert_ne!(divider.bg, selection_bg, "divider flooded");
    }

    #[test]
    fn hovered_journal_card_lifts_to_the_hover_background() {
        pin_flat();
        let theme = theme::test_flat_theme();
        let mut app = app_with_journals(&["work", "zeta"]);
        app.hover = crate::tui::state::HoverTarget::Journal(1);
        let layout = tui_layout(Rect::new(0, 0, 120, 30), &app);
        let list = journal_list_rect(layout.journals.unwrap().content);
        let backend = render_app(app, 120, 30);
        let buffer = backend.buffer();
        // The second (unselected) card sits one row block down; hovered, its
        // background is the hover lift instead of the element surface.
        let hovered = &buffer[(list.x + 2, list.y + 5)];
        assert_eq!(hovered.bg, theme.hover().bg.unwrap());
        // The selected card keeps its selection background.
        let selected = &buffer[(list.x + 2, list.y + 1)];
        assert_eq!(selected.bg, theme.selection().bg.unwrap());
    }

    #[test]
    fn entry_cards_sit_on_the_element_surface_with_plain_spacers() {
        pin_flat();
        let theme = theme::test_flat_theme();
        let mut app = app_with_entries(2);
        app.nav.selected_entry_index = None;
        let layout = tui_layout(Rect::new(0, 0, 120, 30), &app);
        let content = layout.entries.unwrap().panel.content;
        let backend = render_app(app, 120, 30);
        let buffer = backend.buffer();

        // Row 0 is the leading blank; the first card starts on row 1.
        let card = &buffer[(content.x + 1, content.y + 1)];
        assert_eq!(card.bg, theme.element_bg());
        // Spacer rows keep the panel surface so the cards read as blocks.
        let spacer = &buffer[(content.x + 1, content.y)];
        assert_ne!(spacer.bg, theme.element_bg());
    }

    #[test]
    fn hovered_entry_box_carries_the_hover_background() {
        pin_flat();
        let theme = theme::test_flat_theme();
        let mut app = app_with_entry();
        // Deselect so the selection highlight doesn't own the hovered row.
        app.nav.selected_entry_index = None;
        app.hover = crate::tui::state::HoverTarget::Entry(0);
        let layout = tui_layout(Rect::new(0, 0, 120, 30), &app);
        let content = layout.entries.unwrap().panel.content;
        let backend = render_app(app, 120, 30);
        let buffer = backend.buffer();
        // Row 0 is the leading blank; the entry's box starts on row 1.
        let cell = &buffer[(content.x + 1, content.y + 1)];
        assert_eq!(cell.bg, theme.hover().bg.unwrap());
    }

    #[test]
    fn hovered_insights_tab_uses_hint_style_text_without_hover_background() {
        pin_flat();
        let theme = theme::test_flat_theme();
        let mut app = app_with_entry();
        focus_insights(&mut app, insights::InsightsTab::Overview);
        app.hover = HoverTarget::InsightsTab(insights::InsightsTab::Writing);
        let layout = tui_layout(Rect::new(0, 0, 140, 30), &app);
        let insights = layout.insights.expect("insights panel");
        let col = (insights.area.x..insights.area.x + insights.area.width)
            .find(|col| {
                insights_tab_at(insights.area, *col, insights.area.y)
                    == Some(insights::InsightsTab::Writing)
            })
            .expect("writing tab");

        let backend = render_app(app, 140, 30);
        let cell = &backend.buffer()[(col, insights.area.y)];
        assert_eq!(cell.fg, theme.text().fg.unwrap());
        assert_ne!(cell.bg, theme.hover().bg.unwrap());
    }

    #[test]
    fn focused_insights_active_tab_uses_the_accent_title_style_not_a_fill() {
        pin_flat();
        let theme = theme::test_flat_theme();
        let mut app = app_with_entry();
        focus_insights(&mut app, insights::InsightsTab::Overview);
        let layout = tui_layout(Rect::new(0, 0, 140, 30), &app);
        let insights = layout.insights.expect("insights panel");
        let col = (insights.area.x..insights.area.x + insights.area.width)
            .find(|col| {
                insights_tab_at(insights.area, *col, insights.area.y)
                    == Some(insights::InsightsTab::Overview)
            })
            .expect("overview tab");

        let backend = render_app(app, 140, 30);
        let cell = &backend.buffer()[(col, insights.area.y)];
        assert_eq!(cell.fg, theme.primary().fg.unwrap());
        assert_ne!(cell.bg, theme.selection().bg.unwrap());
    }

    #[test]
    fn entry_cards_embed_the_border_labels_inside_padding() {
        pin_flat();
        let flat = rendered_lines(&entry_box_lines(
            Some("Sunday 05"),
            "14:30",
            "hello world",
            Some("2 words ★"),
            Some("Archived"),
            40,
        ));
        crate::tui::theme::set_test_theme(crate::tui::theme::Theme::terminal_default());
        let bordered = rendered_lines(&entry_box_lines(
            Some("Sunday 05"),
            "14:30",
            "hello world",
            Some("2 words ★"),
            Some("Archived"),
            40,
        ));

        // The flat card pads one blank row above the header and below the
        // footer so the labels sit off the card's edge; heights are per-row
        // metadata, so differing from the bordered box is fine.
        assert_eq!(flat.len(), bordered.len() + 2);
        assert_eq!(flat.first().unwrap().trim(), "");
        assert_eq!(flat.last().unwrap().trim(), "");

        // No border glyphs anywhere in the card.
        for line in &flat {
            assert!(
                !line.contains(['┌', '┐', '└', '┘', '│', '─']),
                "border glyph left in flat card line {line:?}"
            );
        }
        // The border labels move into the header and footer rows.
        assert!(flat[1].starts_with("  Sunday 05"));
        assert!(flat[1].trim_end().ends_with("14:30"));
        let footer = &flat[flat.len() - 2];
        assert!(footer.starts_with("  2 words ★"));
        assert!(footer.trim_end().ends_with("Archived"));
    }

    #[test]
    fn bordered_dialogs_on_colored_themes_carry_the_dialog_surface() {
        // A colored theme forced into bordered chrome must not fall back to
        // the terminal-default background inside its dialogs (`Clear` alone
        // would). Classic is unaffected: its panel is the terminal default.
        theme::set_test_theme(theme::test_flat_theme());
        theme::set_chrome_override(Some(crate::tui::theme::ChromeStyle::Bordered));
        let dialog_bg = theme::test_flat_theme().dialog_bg();
        let backend = render_backend(80, 24, |frame| {
            dialogs::draw_edit_metadata_dialog(
                frame,
                &mut tags_state(),
                crate::tui::state::HoverTarget::None,
            )
        });
        let area = metadata_dialog_layout(Rect::new(0, 0, 80, 24), 2).area;
        let border = &backend.buffer()[(area.x, area.y)];
        assert_eq!(border.symbol(), "┌", "chrome override not applied");
        assert_eq!(
            border.fg,
            theme::test_flat_theme().dialog_border().fg.unwrap(),
            "dialog frame fell back to terminal-default ink"
        );
        let interior = &backend.buffer()[(area.x + 1, area.y + 1)];
        assert_eq!(interior.bg, dialog_bg);
    }

    #[test]
    fn dialogs_repaint_the_theme_ink_after_clearing() {
        // `Clear` resets cells to the terminal's own colors; the dialog frame
        // must re-establish the theme's text fg along with its surface, or
        // unstyled dialog text renders in the terminal's ink — near-white on a
        // light-mode dialog on a dark terminal.
        for chrome in [
            None,
            Some(crate::tui::theme::ChromeStyle::Flat),
            Some(crate::tui::theme::ChromeStyle::Bordered),
        ] {
            theme::set_test_theme(theme::test_flat_theme());
            theme::set_chrome_override(chrome);
            let ink = theme::theme().text().fg.expect("flat theme has body ink");
            let backend = render_backend(80, 24, |frame| {
                frames::draw_dialog_frame(frame, Rect::new(10, 5, 40, 10), "Title", false);
            });
            let interior = &backend.buffer()[(12, 10)];
            assert_eq!(
                interior.fg, ink,
                "dialog interior lost the theme ink ({chrome:?})"
            );
        }
        theme::set_chrome_override(None);
    }

    #[test]
    fn entry_body_cache_rebuilds_when_the_theme_changes() {
        // The rendered body bakes in markdown colors, glyphs, and syntax
        // highlighting; the picker's live preview swaps themes without
        // touching the entry, so the memo key must notice.
        theme::set_test_theme(theme::test_flat_theme());
        let app = app_with_journals(&["alpha"]);
        let builds = std::cell::Cell::new(0);
        let build = || {
            builds.set(builds.get() + 1);
            crate::tui::app::RenderedEntryBody::default()
        };
        app.cached_entry_body(None, 80, build);
        app.cached_entry_body(None, 80, build);
        assert_eq!(builds.get(), 1, "same theme must hit the cache");
        theme::set_test_theme(theme::test_eclipse_theme());
        app.cached_entry_body(None, 80, build);
        assert_eq!(builds.get(), 2, "a theme change must rebuild the body");
    }

    #[test]
    fn bordered_key_chips_carry_the_key_hint_token() {
        // Bordered chrome used to hardcode REVERSED|BOLD chips; the token's
        // default is that same chip, so classic is a no-op — but a flat theme
        // forced to bordered must keep its own key_hint ink.
        theme::set_test_theme(theme::test_flat_theme());
        theme::set_chrome_override(Some(crate::tui::theme::ChromeStyle::Bordered));
        let theme = theme::test_flat_theme();
        let app = app_with_journals(&["alpha"]);
        let text = footer::footer_lines(&app, 120);
        let spans: Vec<_> = text
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .collect();
        let label = spans
            .iter()
            .position(|span| span.content == " quit")
            .expect("quit label in the browse footer");
        let chip = spans[label - 1];
        assert_eq!(chip.style, theme.key_hint(), "chip ignored the token");
        theme::set_chrome_override(None);
    }

    #[test]
    fn scrollbars_carry_the_scrollbar_tokens_on_both_chromes() {
        use ratatui::widgets::ScrollbarState;
        theme::set_test_theme(theme::test_flat_theme());
        let theme = theme::test_flat_theme();
        for (chrome_override, scrollbar_x) in [
            (crate::tui::theme::ChromeStyle::Flat, 2),
            (crate::tui::theme::ChromeStyle::Bordered, 3),
        ] {
            theme::set_chrome_override(Some(chrome_override));
            let backend = render_backend(4, 12, |frame| {
                let mut state = ScrollbarState::default()
                    .content_length(100)
                    .viewport_content_length(10)
                    .position(0);
                chrome::render_vertical_scrollbar(frame, frame.area(), &mut state, true);
            });
            // One vertical margin row and an arrow on each end; the thumb hugs
            // the top at position 0 and the track fills the rest.
            let thumb = &backend.buffer()[(scrollbar_x, 2u16)];
            let track = &backend.buffer()[(scrollbar_x, 9u16)];
            assert_eq!(thumb.fg, theme.scrollbar_thumb(true).fg.unwrap());
            assert_eq!(track.fg, theme.scrollbar_track(true).fg.unwrap());
        }
        theme::set_chrome_override(None);
    }

    #[test]
    fn themed_border_set_draws_panels_cards_and_tables() {
        theme::set_test_theme(theme::test_theme_from_toml(
            "[borders]\nstyle = \"rounded\"",
        ));
        let corner = |focused: bool| {
            let backend = render_backend(20, 5, move |frame| {
                frame.render_widget(chrome::panel_block("t", focused, None), frame.area());
            });
            backend.buffer()[(0u16, 0u16)].symbol().to_string()
        };
        assert_eq!(corner(false), "╭", "unfocused panel ignored the set");
        assert_eq!(corner(true), "┏", "focus must stay thick");

        theme::set_test_theme(theme::test_theme_from_toml("[borders]\nstyle = \"ascii\""));
        let text = |line: ratatui::text::Line| -> String {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        };
        assert_eq!(
            text(crate::tui::entry_rows::border_line(
                crate::tui::entry_rows::BoxEdge::Top,
                10,
                None,
                None,
            )),
            "+--------+"
        );
        assert_eq!(
            text(table::rule(
                &[3],
                table::RulePos::Top,
                ratatui::style::Style::default(),
                ratatui::style::Style::default(),
            )),
            "+-----+"
        );
    }

    #[test]
    fn bordered_chrome_styles_unfocused_panel_borders_with_the_theme() {
        // A flat-designed theme forced into bordered chrome must not draw
        // inactive panel borders in the terminal-default ink — that reads
        // *brighter* than the focused border on a muted palette. Classic is
        // unaffected: its `border_inactive` is the terminal default.
        theme::set_test_theme(theme::test_flat_theme());
        theme::set_chrome_override(Some(crate::tui::theme::ChromeStyle::Bordered));
        let theme = theme::test_flat_theme();
        let corner = |focused: bool| {
            let backend = render_backend(20, 5, move |frame| {
                frame.render_widget(chrome::panel_block("t", focused, None), frame.area());
            });
            backend.buffer()[(0u16, 0u16)].clone()
        };
        assert_eq!(corner(false).fg, theme.inactive_border().fg.unwrap());
        assert_eq!(corner(true).fg, theme.focus_border().fg.unwrap());
        theme::set_chrome_override(None);
    }

    #[test]
    fn hovered_dialog_row_lifts_even_when_it_is_the_hidden_selection() {
        pin_flat();
        let theme = theme::test_flat_theme();
        // Focus on the input: the list's selection highlight is hidden, so the
        // selected row (index 0, the default) must still respond to hover.
        let mut state = tags_state();
        state.focus = crate::tui::app::EditMetadataFocus::Input;
        let layout = metadata_dialog_layout(Rect::new(0, 0, 80, 24), 2);
        let backend = render_backend(80, 24, |frame| {
            dialogs::draw_edit_metadata_dialog(
                frame,
                &mut state,
                crate::tui::state::HoverTarget::DialogRow(0),
            )
        });
        let cell = &backend.buffer()[(layout.list.x + 3, layout.list.y)];
        assert_eq!(cell.bg, theme.hover().bg.unwrap());
    }

    #[test]
    fn hovered_footer_hint_label_lifts_out_of_the_muted_row() {
        pin_flat();
        let theme = theme::test_flat_theme();
        let mut app = app_with_journals(&["alpha"]);
        app.hover = crate::tui::state::HoverTarget::FooterHint(footer::HintId::Quit);
        let text = footer::footer_lines(&app, 120);
        let label = text
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content == " quit")
            .expect("quit label in the browse footer");
        assert_eq!(label.style, theme.text(), "hovered label still muted");
    }

    #[test]
    fn bordered_footer_hint_labels_keep_flat_text_styles() {
        pin_flat();
        theme::set_chrome_override(Some(crate::tui::theme::ChromeStyle::Bordered));
        let theme = theme::test_flat_theme();
        let mut app = app_with_journals(&["alpha"]);
        let label_style = |app: &App| {
            footer::footer_lines(app, 120)
                .lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .find(|span| span.content == " quit")
                .expect("quit label in the browse footer")
                .style
        };

        assert_eq!(label_style(&app), theme.muted());
        app.hover = crate::tui::state::HoverTarget::FooterHint(footer::HintId::Quit);
        assert_eq!(label_style(&app), theme.text());
        theme::set_chrome_override(None);
    }

    #[test]
    fn footer_key_chips_are_not_reversed() {
        pin_flat();
        let app = app_with_journals(&["alpha"]);
        let backend = render_app(app, 120, 30);
        let buffer = backend.buffer();
        for y in 0..30u16 {
            for x in 0..120u16 {
                assert!(
                    !buffer[(x, y)].modifier.contains(Modifier::REVERSED),
                    "reversed cell at ({x},{y}) in flat chrome"
                );
            }
        }
    }

    #[test]
    fn flat_dialogs_pad_the_title_and_footer_off_the_card_edge() {
        pin_flat();
        // Regular dialog: blank row, then the title row.
        let backend = render_backend(80, 24, |frame| {
            dialogs::draw_edit_metadata_dialog(
                frame,
                &mut tags_state(),
                crate::tui::state::HoverTarget::None,
            )
        });
        let area = metadata_dialog_layout(Rect::new(0, 0, 80, 24), 2).area;
        let row_text = |y: u16| -> String {
            (area.x..area.x + area.width)
                .map(|x| backend.buffer()[(x, y)].symbol())
                .collect()
        };
        assert_eq!(row_text(area.y).trim(), "", "top padding row not blank");
        assert!(row_text(area.y + 1).contains("Edit Tags"));
        assert_eq!(
            row_text(area.y + 2).trim(),
            "",
            "no blank row under the title"
        );
        assert_eq!(
            row_text(area.y + area.height - 1).trim(),
            "",
            "bottom padding row not blank"
        );

        // Table dialog: the footer sits above the bottom padding row.
        let text_rows = render_to_rows(64, 20, |frame| menus::draw_settings_menu(frame, None));
        let footer_row = text_rows
            .iter()
            .position(|row| row.contains("esc close"))
            .expect("settings footer");
        assert_eq!(
            text_rows[footer_row + 1].trim(),
            "",
            "no padding under the footer"
        );
        let title_row = text_rows
            .iter()
            .position(|row| row.contains("Settings"))
            .expect("settings title");
        assert_eq!(
            text_rows[title_row - 1].trim(),
            "",
            "no padding above the title"
        );
    }

    #[test]
    fn dialog_inner_uses_the_shared_surface_gutter_in_both_chromes() {
        pin_flat();
        let area = Rect::new(10, 5, 44, 20);
        for (chrome, expected) in [
            (
                crate::tui::theme::ChromeStyle::Flat,
                Rect::new(12, 8, 39, 16),
            ),
            (
                crate::tui::theme::ChromeStyle::Bordered,
                Rect::new(12, 6, 40, 18),
            ),
        ] {
            theme::set_chrome_override(Some(chrome));
            let inner = frames::dialog_inner(area);
            assert_eq!(inner, expected);
            assert_eq!(inner.x, area.x + 2);
            assert_eq!(inner.x + inner.width, scrollbar_bar_rect(area).x - 1);
        }
        theme::set_chrome_override(None);
    }

    #[test]
    fn toast_renders_top_right_with_variant_edges() {
        pin_flat();
        let mut app = app_with_journals(&["alpha"]);
        app.toast(crate::tui::state::ToastVariant::Success, "Entry saved");

        let backend = render_app(app, 120, 30);
        let buffer = backend.buffer();

        // Width 44 with a right inset of 2 → columns 74..=117, starting at row 1;
        // a one-line message makes a 3-row box.
        let success = theme::test_flat_theme().success();
        for y in 1..=3u16 {
            for x in [74u16, 117u16] {
                assert_eq!(buffer[(x, y)].symbol(), "┃", "edge missing at ({x},{y})");
                assert_eq!(buffer[(x, y)].fg, success.fg.unwrap());
            }
        }
        let message_row: String = (75..117)
            .map(|x| buffer[(x as u16, 2u16)].symbol())
            .collect();
        assert!(
            message_row.contains("Entry saved"),
            "row was: {message_row}"
        );
        // The card sits on the element surface — one step off the panels it
        // floats over, so it separates without relying on the edge stripes.
        assert_eq!(
            buffer[(80u16, 2u16)].bg,
            theme::test_flat_theme().element_bg()
        );
        // Padding rows above and below the message stay blank.
        for y in [1u16, 3u16] {
            let row: String = (75..117).map(|x| buffer[(x as u16, y)].symbol()).collect();
            assert_eq!(row.trim(), "");
        }
    }

    #[test]
    fn toasts_stack_with_a_blank_row_between() {
        pin_flat();
        let mut app = app_with_journals(&["alpha"]);
        app.toast(crate::tui::state::ToastVariant::Info, "First");
        app.toast(crate::tui::state::ToastVariant::Error, "Second");

        let backend = render_app(app, 120, 30);
        let buffer = backend.buffer();

        // Oldest on top (rows 1..=3), a blank row, then the newest (rows 5..=7).
        let info = theme::test_flat_theme().info();
        let error = theme::test_flat_theme().error();
        assert_eq!(buffer[(74u16, 1u16)].fg, info.fg.unwrap());
        assert_ne!(buffer[(74u16, 4u16)].symbol(), "┃");
        assert_eq!(buffer[(74u16, 5u16)].symbol(), "┃");
        assert_eq!(buffer[(74u16, 5u16)].fg, error.fg.unwrap());
    }

    // Flat chrome has no top border for the box title to fold into, so the title
    // takes its own inner row. These pin that the container is sized for it and
    // the last line — the command / hint — stays inside the box.

    #[test]
    fn pending_notice_shows_the_enroll_command_in_flat_chrome() {
        pin_flat();
        let text =
            render_pending_notice_text("phone", &AccessNotice::NeedsEnroll { retired_key: false });
        assert!(text.contains("Not authorized"));
        assert!(text.contains(crate::ENROLL_CMD));
    }

    #[test]
    fn pending_notice_shows_the_approve_command_in_flat_chrome() {
        pin_flat();
        let text = render_pending_notice_text("phone", &AccessNotice::AwaitingApproval);
        assert!(text.contains("Awaiting approval"));
        assert!(text.contains(&format!("{} phone", crate::APPROVE_CMD)));
    }

    #[test]
    fn disable_notice_shows_the_enable_hint_in_flat_chrome() {
        pin_flat();
        let backend = render_backend(72, 20, draw_disable_notice);
        let text: String = backend
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Encryption disabled"));
        assert!(text.contains("notema encryption enable"));
    }
}

mod toast_bordered_tests {
    use super::*;

    #[test]
    fn toast_draws_a_variant_colored_border_box() {
        // The default (terminal/classic) theme is bordered chrome.
        let mut app = app_with_journals(&["alpha"]);
        app.toast(crate::tui::state::ToastVariant::Error, "Save failed");

        let backend = render_app(app, 120, 30);
        let buffer = backend.buffer();

        assert_eq!(buffer[(74u16, 1u16)].symbol(), "┌");
        assert_eq!(buffer[(117u16, 1u16)].symbol(), "┐");
        assert_eq!(buffer[(74u16, 3u16)].symbol(), "└");
        if let Some(fg) = crate::tui::theme::theme().error().fg {
            assert_eq!(buffer[(74u16, 1u16)].fg, fg);
        }
        let message_row: String = (75..117)
            .map(|x| buffer[(x as u16, 2u16)].symbol())
            .collect();
        assert!(
            message_row.contains("Save failed"),
            "row was: {message_row}"
        );
    }
}

mod scrim_tests {
    use super::*;
    use crate::tui::theme;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Style};

    #[test]
    fn scrim_blends_rgb_cells_and_dims_the_rest() {
        theme::set_test_theme(theme::test_flat_theme()); // scrim strength 0.45
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 0, "abc", Style::default());
        buf[(0u16, 0u16)].fg = Color::Rgb(0x80, 0x80, 0x80);
        buf[(0u16, 0u16)].bg = Color::Rgb(0x10, 0x10, 0x10);
        buf[(1u16, 0u16)].fg = Color::Reset;
        buf[(2u16, 0u16)].set_diff_option(ratatui::buffer::CellDiffOption::Skip);

        chrome::scrim(&mut buf, area);

        // 0x80 * (1 - 0.45) = 0x46; the blended cell gains no DIM.
        assert_eq!(buf[(0u16, 0u16)].fg, Color::Rgb(0x46, 0x46, 0x46));
        assert_eq!(buf[(0u16, 0u16)].bg, Color::Rgb(0x08, 0x08, 0x08));
        assert!(!buf[(0u16, 0u16)].modifier.contains(Modifier::DIM));
        // Non-RGB cells fall back to the DIM modifier.
        assert!(buf[(1u16, 0u16)].modifier.contains(Modifier::DIM));
        // Graphics-protocol cells are untouched.
        assert!(!buf[(2u16, 0u16)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn scrim_at_zero_strength_only_dims() {
        // The default terminal theme has scrim 0: every cell gets DIM, colors
        // stay untouched.
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        buf[(0u16, 0u16)].fg = Color::Rgb(0x80, 0x80, 0x80);

        chrome::scrim(&mut buf, area);

        assert_eq!(buf[(0u16, 0u16)].fg, Color::Rgb(0x80, 0x80, 0x80));
        assert!(buf[(0u16, 0u16)].modifier.contains(Modifier::DIM));
    }
}
