use super::*;
use crate::{
    config::Config,
    tui::{
        app::{Focus, INLINE_ENTRY_VIEW_MIN_WIDTH, Mode},
        state::{EditMetadataFocus, EditMetadataState, MetadataKind},
        test_support::{app_with_entry, app_with_journals, new_app},
    },
};
use journal_storage::{Entry, EntryEncryptionState, SearchHit};
use ratatui::{Frame, Terminal, backend::TestBackend, layout::Rect, style::Modifier, text::Line};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use unicode_width::UnicodeWidthStr;

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
        dialogs::draw_edit_metadata_dialog(frame, &mut state)
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
        location: &[],
    }
}

fn render_confirm_delete_rows(width: u16, height: u16) -> Vec<String> {
    render_to_rows(width, height, |frame| {
        dialogs::draw_confirm_delete(
            frame,
            &crate::tui::state::DeleteContext::Entry { has_body: true },
        )
    })
}

#[test]
fn layout_places_hit_targets_in_three_columns() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, 140, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.entry_view.is_some());
    assert!(layout.insights.is_none());
    assert_eq!(layout.journals.unwrap().area, Rect::new(0, 0, 27, 19));
    assert_eq!(layout.entries.unwrap().panel.area, Rect::new(27, 0, 47, 19));
    assert_eq!(layout.entry_view.unwrap().area, Rect::new(74, 0, 66, 19));
    assert_eq!(layout.footer, Rect::new(0, 19, 140, 1));
}

#[test]
fn layout_keeps_three_columns_at_minimum_inline_width() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, INLINE_ENTRY_VIEW_MIN_WIDTH, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.entry_view.is_some());
    assert!(layout.insights.is_none());
    let ch = 20 - footer_height(&app, INLINE_ENTRY_VIEW_MIN_WIDTH);
    assert_eq!(layout.journals.unwrap().area, Rect::new(0, 0, 27, ch));
    assert_eq!(layout.entries.unwrap().panel.area, Rect::new(27, 0, 47, ch));
    assert_eq!(layout.entry_view.unwrap().area, Rect::new(74, 0, 51, ch));
}

#[test]
fn layout_places_hit_targets_in_two_columns_without_inline_entry_view() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Journals;

    let layout = tui_layout(Rect::new(0, 0, 90, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.entry_view.is_none());
    assert!(layout.insights.is_none());
    let ch = 20 - footer_height(&app, 90);
    assert_eq!(layout.journals.unwrap().area, Rect::new(0, 0, 27, ch));
    assert_eq!(layout.entries.unwrap().panel.area, Rect::new(27, 0, 63, ch));
}

#[test]
fn layout_shifts_two_columns_to_entries_and_preview_when_entries_are_active() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::Entries;

    let layout = tui_layout(Rect::new(0, 0, 90, 20), &app);

    assert!(!layout.single_panel);
    assert!(layout.entry_view.is_some());
    assert!(layout.insights.is_none());
    assert!(layout.journals.is_none());
    let content_height = 20 - footer_height(&app, 90);
    assert_eq!(
        layout.entries.unwrap().panel.area,
        Rect::new(0, 0, 47, content_height)
    );
    assert_eq!(
        layout.entry_view.unwrap().area,
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
    let location = vec!["Testville, Testland".to_string()];
    let values = EntryMetadataValues {
        tags: &tags,
        people: &[],
        activities: &[],
        feelings: &[],
        mood: None,
        location: &location,
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
            "+++\ntags = [\"work\", \"personal\", \"health\"]\nfeelings = [\"calm\", \"focused\", \"tired\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
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
                "+++\ntags = [\"tiny-screen\"]\nfeelings = [\"focused\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\n{body}\n",
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
    let wide_tags = metadata_dialog_layout(Rect::new(0, 0, 120, 30), 20);
    assert_eq!(wide_tags.area.width, 44);
    assert_eq!(wide_tags.list.height, 14);

    let narrow_tags = metadata_dialog_layout(Rect::new(0, 0, 40, 30), 20);
    assert_eq!(narrow_tags.area.x, 0);
    assert_eq!(narrow_tags.area.width, 40);

    // 15 group headers + 155 feelings = 170 rows; the list caps at its max visible rows.
    let wide_feelings = feelings_dialog_layout(Rect::new(0, 0, 120, 30), 170, 1);
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
    use crate::tui::state::EditFeelingState;
    use journal_core::feelings::{Feeling, FeelingGroup};

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
        dialogs::draw_edit_feelings_dialog(frame, &mut state)
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
    use crate::tui::state::EditFeelingState;
    use journal_core::feelings::{Feeling, FeelingGroup};

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
        dialogs::draw_edit_feelings_dialog(frame, &mut state)
    });
    assert!(
        rows.iter().any(|row| row.contains("(no matches)")),
        "an empty filter must still surface the no-matches line"
    );
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

    assert!(rendered.contains(">[ ] tag-00 (0)"));
    assert!(rendered.contains("toggle (space)"));
    assert!(rendered.contains("input (tab)"));
    assert!(rendered.contains("save (enter)"));
    assert!(rendered.contains("cancel (esc)"));
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
    state.input = "missing".to_string();
    state.focus = EditMetadataFocus::Input;
    let rendered = render_edit_tags_dialog_text(state, 200, 12);

    assert!(rendered.contains(" (no matches)"));
    assert!(rendered.contains(" add (enter) | list (tab) | cancel (esc)"));
}

#[test]
fn edit_metadata_input_hint_saves_when_empty_and_adds_when_not_empty() {
    let mut empty =
        EditMetadataState::new(MetadataKind::People, Vec::new(), Vec::new(), Vec::new(), 0);
    empty.focus = EditMetadataFocus::Input;
    let rendered_empty = render_edit_tags_dialog_text(empty, 200, 12);
    assert!(rendered_empty.contains(" save (enter) | list (tab) | cancel (esc)"));

    let mut with_value =
        EditMetadataState::new(MetadataKind::People, Vec::new(), Vec::new(), Vec::new(), 0);
    with_value.focus = EditMetadataFocus::Input;
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
        "+++\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nFirst preview\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\n[datetime]\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nSecond preview\n",
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
            format!("+++\n[datetime]\ncreated_at = \"{ts}\"\n+++\n\n# e{index}\nBody text\n"),
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
            "+++\nfeelings = [\"calm\", \"focused\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
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
            "+++\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\n```mermaid\n  graph TD\n      A[Open journal] --> B[Write entry]\n      B --> C{Preview}\n      C -->|looks good| D[Save]\n      C -->|needs work| B\n  ```\n",
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
        "+++\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    fs::write(
        work_entry_dir.join("b.md"),
        "+++\n[datetime]\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nBody\n",
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
fn multi_col_fullscreen_takes_the_whole_width_and_hides_columns() {
    let mut app = app_with_entry();
    app.nav.focus = Focus::EntryView;
    app.nav.entry_view_fullscreen = true;

    let layout = tui_layout(Rect::new(0, 0, 130, 20), &app);
    assert!(layout.journals.is_none());
    assert!(layout.entries.is_none());
    assert_eq!(layout.entry_view.unwrap().area.width, 130);

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
        "+++\ntags = [\"work\"]\nfeelings = [\"calm\"]\npeople = [\"alex\"]\nactivities = [\"running\"]\nmood = 3\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    let config = Config::new(dir.path().to_path_buf(), "true");
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
                "+++\n{meta}\nmood = {mood}\n[datetime]\ncreated_at = \"2026-07-{day:02}T10:00:00+02:00\"\n+++\n\n# E\nBody\n"
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
                "+++\n{meta}mood = {mood}\n[datetime]\ncreated_at = \"2026-07-{day:02}T10:00:00+02:00\"\n+++\n\n# A\nBody\n"
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
    // Single-column full screen (the flag is unset): Left also exits, so it is
    // listed alongside Enter/Esc.
    assert!(expanded_text.contains("close (enter/esc/←)"));
    assert!(expanded_text.contains("edit (e) | close (enter/esc/←) | del (d)"));

    // Multi-column full screen: Left is inert (Esc collapses), so it drops from the
    // close hint.
    app.nav.entry_view_fullscreen = true;
    assert!(expanded_footer_text(&app).contains("close (enter/esc)"));
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
        starred: false,
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
    let hints = metadata_dialog_hints(EditMetadataFocus::List, true);

    assert_eq!(hint_height(hints, 29), 2);
    assert_eq!(
        hint_id_at_wrapped(hints, 10, 5, 29, 10, 6),
        Some(HintId::MetadataSave)
    );
}

#[test]
fn dialog_hint_routing_uses_typed_ids() {
    let tags = metadata_dialog_hints(EditMetadataFocus::List, true);
    assert_eq!(hint_id_at(tags, 10, 11), Some(HintId::MetadataToggle));

    let empty_input = metadata_dialog_hints(EditMetadataFocus::Input, true);
    assert_eq!(hint_id_at(empty_input, 10, 11), Some(HintId::MetadataSave));

    let value_input = metadata_dialog_hints(EditMetadataFocus::Input, false);
    assert_eq!(
        hint_id_at(value_input, 10, 11),
        Some(HintId::MetadataAddFromInput)
    );

    let feelings = feelings_dialog_hints(EditMetadataFocus::List);
    assert_eq!(
        hint_id_at(
            feelings,
            20,
            20 + UnicodeWidthStr::width("open (→) | close (←) | toggle (space) | search (tab) | ")
                as u16
        ),
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
        starred: false,
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
        starred: false,
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
        edited_at: None,
        preview: preview.to_string(),
        metadata: journal_storage::Metadata::default(),
        location: None,
        import: None,
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
        created_at: Some(journal_storage::Timestamp::parse(
            "2026-07-01T10:23:00+02:00",
        )),
        edited_at: None,
        preview: String::new(),
        metadata: journal_storage::Metadata::default(),
        location: None,
        import: None,
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
        edited_at: None,
        preview: String::new(),
        metadata: journal_storage::Metadata::default(),
        location: None,
        import: None,
        content: String::new(),
        word_count: 0,
        search_haystack: String::new(),
    };

    assert_eq!(entry_month_label(&entry), Some("July 2026".to_string()));
    assert_eq!(entry_day_label(&entry), Some("Wednesday 01".to_string()));
}

fn render_unlock_text(input: &str, error: Option<&str>, caret_visible: bool) -> String {
    render_to_text(60, 16, |frame| {
        draw_unlock(frame, input, error, caret_visible)
    })
}

#[test]
fn unlock_screen_masks_passphrase_and_draws_border() {
    let text = render_unlock_text("hunter2", None, false);
    // Bordered fullscreen chrome with the title and hint.
    assert!(text.contains("Unlock Journal"));
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
    let text = render_unlock_text("", Some("Incorrect passphrase"), true);
    // The error takes the hint's place after a wrong passphrase.
    assert!(text.contains("Incorrect passphrase"));
    assert!(!text.contains("Enter your passphrase to unlock"));
}

fn render_unlock_rows(width: u16, height: u16, error: Option<&str>) -> Vec<String> {
    render_to_rows(width, height, |frame| draw_unlock(frame, "", error, false))
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
    // Outer journal chrome frame with its dismiss hint, plus the inner state box.
    assert!(text.contains("Journal"));
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
    assert!(text.contains("Journal"));
    assert!(text.contains("any key to continue"));
    assert!(text.contains("Encryption disabled"));
    assert!(text.contains("journal encryption enable"));
}
