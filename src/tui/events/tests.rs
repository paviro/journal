use super::*;
use crate::{
    config::Config,
    tui::{
        app::{App, EditMetadataFocus, FeelingRow, Focus, LocationPreset, ScrollbarDrag},
        render,
        render::insights::{InsightsTab, InsightsTimeframe},
        scroll,
        state::{HoverTarget, ListNav},
        test_support::{app_with_entries, app_with_journals, new_app},
    },
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
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

fn mouse_in_area(app: &mut App, event: MouseEvent, w: u16, h: u16) {
    mouse::handle_mouse_in_area(app, event, Rect::new(0, 0, w, h)).unwrap();
}

fn set_tag_dialog_items(app: &mut App, count: usize) {
    let state = app.edit_metadata_state_mut().unwrap();
    state.all_values = (0..count)
        .map(|index| (format!("tag-{index:02}"), index + 1))
        .collect();
    state.filtered = (0..count).collect();
    state.normalize_list_state();
}

#[test]
fn enter_on_journals_moves_to_entries_like_right_arrow() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("work")).unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut enter_app = new_app(config.clone());
    let mut right_app = new_app(config);

    enter_app.nav.focus = Focus::Journals;
    right_app.nav.focus = Focus::Journals;

    // Enter and Right on Journals both resolve to move_focus_right
    move_focus_right(&mut enter_app, true);
    move_focus_right(&mut right_app, true);

    assert_eq!(enter_app.nav.focus, Focus::Entries);
    assert_eq!(enter_app.nav.focus, right_app.nav.focus);
}

#[test]
fn right_on_entry_expands_when_inline_entry_view_is_hidden() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\nBody\n").unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;

    // Right on Entries when not entry_view_available → ViewSelected → view_selected
    view_selected(&mut app).unwrap();

    assert_eq!(app.nav.focus, Focus::EntryView);
}

#[test]
fn expanded_entry_title_matches_entry_view_timestamp_title() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\n[datetime]\ncreated_at = \"2026-07-01T10:23:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;

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
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;

    // Right on Entries when entry_view_available → FocusRight → focus to EntryView
    move_focus_right(&mut app, true);

    assert_eq!(app.nav.focus, Focus::EntryView);
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn keyboard_and_footer_edit_use_the_same_action() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;

    let key_action = keyboard::key_to_action(&app, key(KeyCode::Char('e')), true);
    let hint_action = mouse::hint_id_to_action(&app, render::HintId::EditSelected);

    assert_eq!(key_action, Some(Action::EditSelected));
    assert_eq!(hint_action, Some(Action::EditSelected));
}

#[test]
fn editor_footer_hints_route_to_editor_actions() {
    let mut app = app_with_entries(1);
    app.open_editor_for_selected();
    app.state.ui.show_hints = false;

    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::EditorSave),
        Some(Action::EditorSave)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::EditorDiscard),
        Some(Action::EditorRequestDiscard)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::EditorMetadata),
        Some(Action::EditorOpenMetadataMenu)
    );
}

#[test]
fn right_past_entries_focuses_insights_and_arrows_cycle_tabs() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;
    // No entry selected → the right column is the insights preview.
    app.nav.selected_entry_index = None;
    assert!(app.show_journal_insights_preview());

    // Right past Entries focuses the panel on its first tab.
    move_focus_right(&mut app, true);
    assert_eq!(app.nav.focus, Focus::Insights);
    assert_eq!(app.nav.insights_tab, InsightsTab::Overview);
    assert!(app.insights_panel_focused());

    // Right steps forward through the tabs without leaving the panel.
    move_focus_right(&mut app, true);
    assert_eq!(app.nav.focus, Focus::Insights);
    assert_eq!(app.nav.insights_tab, InsightsTab::Writing);

    // Left steps back; from the first tab it leaves to the entries column.
    move_focus_left(&mut app);
    assert_eq!(
        (app.nav.focus, app.nav.insights_tab),
        (Focus::Insights, InsightsTab::Overview)
    );
    move_focus_left(&mut app);
    assert_eq!(app.nav.focus, Focus::Entries);
}

#[test]
fn right_reaches_insights_in_single_panel_layout() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;
    // No entry selected → the entries column previews the journal insights.
    app.nav.selected_entry_index = None;
    assert!(app.show_journal_insights_preview());

    // At single-panel width (entry view unavailable) Right still focuses the panel,
    // which renders full-screen; Left from the first tab returns to the entries list.
    move_focus_right(&mut app, false);
    assert_eq!(app.nav.focus, Focus::Insights);
    assert_eq!(app.nav.insights_tab, InsightsTab::Overview);

    move_focus_left(&mut app);
    assert_eq!(app.nav.focus, Focus::Entries);
}

#[test]
fn enter_expands_and_collapses_the_insights_panel() {
    let mut app = app_with_entries(1);
    app.nav.selected_entry_index = None;
    app.nav.focus = Focus::Insights;

    // Enter on the focused panel expands it; Enter/Esc collapse it back.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::ExpandInsights)
    );
    app.nav.insights_fullscreen = true;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::CollapseInsights)
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Esc), true),
        Some(Action::CollapseInsights)
    );

    // Leaving the panel (Left from the first tab) resets full-screen so it
    // re-opens collapsed.
    move_focus_left(&mut app);
    assert_eq!(app.nav.focus, Focus::Entries);
    assert!(!app.nav.insights_fullscreen);
}

#[test]
fn scope_key_toggles_only_while_insights_panel_is_focused() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Journals;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('g')), true),
        None
    );

    app.nav.focus = Focus::Insights;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('g')), true),
        Some(Action::ToggleInsightsScope)
    );
}

#[test]
fn window_key_cycles_timeframe_only_on_driver_tabs() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Insights;

    // Overview doesn't window, so `w` is inert there.
    app.nav.insights_tab = InsightsTab::Overview;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('w')), true),
        None
    );

    // On Drivers it cycles the rolling window forward, wrapping back to Overall.
    app.nav.insights_tab = InsightsTab::Drivers;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('w')), true),
        Some(Action::CycleInsightsTimeframe)
    );
    assert_eq!(InsightsTimeframe::Overall.next(), InsightsTimeframe::Year);
    assert_eq!(InsightsTimeframe::Week.next(), InsightsTimeframe::Overall);
}

#[test]
fn clicking_a_border_tab_focuses_the_panel_and_selects_that_tab() {
    let mut app = app_with_entries(1);
    // Preview state so the insights panel is the right-hand column.
    app.nav.selected_entry_index = None;
    app.nav.focus = Focus::Journals;

    // Click the "Drivers" label in the insights panel's top border. At width 160 the
    // panel is wide enough for full titles; Drivers is the fourth (last) tab, at x≈117.
    mouse_in_area(&mut app, mouse(down(), 117, 0), 160, 20);

    assert_eq!(app.nav.focus, Focus::Insights);
    assert_eq!(app.nav.insights_tab, InsightsTab::Drivers);
}

#[test]
fn multi_col_enter_focuses_then_expands_then_collapses() {
    let mut app = app_with_entries(1);

    // First Enter opens the focused preview pane (not full screen yet).
    view_selected(&mut app).unwrap();
    assert_eq!(app.nav.focus, Focus::EntryView);
    assert!(!app.nav.entry_view_fullscreen);

    // Second Enter expands to full screen.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::ExpandEntryView)
    );
    app.nav.entry_view_fullscreen = true;

    // Third Enter closes full screen (collapses back to the focused pane).
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::CollapseEntryView)
    );
}

#[test]
fn multi_col_fullscreen_esc_collapses_and_left_is_inert() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();
    app.nav.entry_view_fullscreen = true;

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Esc), true),
        Some(Action::CollapseEntryView)
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Left), true),
        None
    );
}

#[test]
fn single_col_viewer_exits_on_enter_esc_and_left() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();

    // In single-column the viewer is full screen by nature; Enter/Esc/Left all exit.
    for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Left] {
        assert_eq!(
            keyboard::key_to_action(&app, key(code), false),
            Some(Action::FocusLeft),
            "{code:?}"
        );
    }
}

#[test]
fn leaving_the_viewer_clears_fullscreen() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();
    app.nav.entry_view_fullscreen = true;

    move_focus_left(&mut app);

    assert_eq!(app.nav.focus, Focus::Entries);
    assert!(!app.nav.entry_view_fullscreen);
}

#[test]
fn browse_l_opens_the_location_dialog() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;
    app.nav.selected_entry_index = Some(0);

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('l')), true),
        Some(Action::BeginEditLocation)
    );
}

#[test]
fn location_dialog_keys_route_by_focus() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;
    app.nav.selected_entry_index = Some(0);
    app.begin_edit_location();

    // Opens focused on the address field (top): chars type in, and Enter looks
    // the query up (nothing resolved yet).
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('x')), true),
        Some(Action::InputKey(key(KeyCode::Char('x'))))
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Tab), true),
        Some(Action::LocationSwitchFocus)
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Esc), true),
        Some(Action::CancelOverlay)
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::LocationResolve)
    );

    // With a preset present, focus can reach the list (Query → Name → List),
    // where Enter picks a row.
    {
        let state = app.edit_location_state_mut().unwrap();
        state.presets.push(LocationPreset {
            label: "Berlin".to_string(),
            location: journal_core::Location {
                city: Some("Berlin".to_string()),
                ..Default::default()
            },
        });
        state.switch_focus(); // Query -> Name
        state.switch_focus(); // Name -> List
        assert_eq!(state.focus, crate::tui::app::EditLocationFocus::List);
    }
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::LocationSelectRow)
    );
}

#[test]
fn location_ctrl_l_grabs_device_and_plain_l_types() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;
    app.nav.selected_entry_index = Some(0);
    app.begin_edit_location();

    // Ctrl+L grabs the device's current location from any focus...
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
    assert_eq!(
        keyboard::key_to_action(&app, ctrl_l, true),
        Some(Action::LocationGrabDevice)
    );
    // ...but a bare 'l' is still text typed into the query field.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('l')), true),
        Some(Action::InputKey(key(KeyCode::Char('l'))))
    );
}

#[test]
fn location_query_enter_saves_once_the_query_is_resolved() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;
    app.nav.selected_entry_index = Some(0);
    app.begin_edit_location();
    {
        let state = app.edit_location_state_mut().unwrap();
        state.focus = crate::tui::app::EditLocationFocus::Query;
        state.query = "52.5, 13.4".into();
        state.query_looked_up = false;
    }

    // Before a lookup, Enter in the address field queries.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::LocationResolve)
    );

    // Once resolved, Enter saves instead of re-querying.
    app.edit_location_state_mut().unwrap().query_looked_up = true;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::LocationSave)
    );
}

#[test]
fn snapshot_restores_fullscreen_across_an_edit() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();
    app.nav.entry_view_fullscreen = true;

    let snapshot = EntryViewSnapshot::capture(&app);
    app.nav.entry_view_fullscreen = false;
    app.nav.focus = Focus::Entries;
    restore_entry_view_or_close(&mut app, snapshot);

    assert_eq!(app.nav.focus, Focus::EntryView);
    assert!(app.nav.entry_view_fullscreen);
}

#[test]
fn typed_hint_ids_route_to_actions_without_string_parsing() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;

    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::OpenMetadataMenu),
        Some(Action::OpenMetadataMenu)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::EditSelected),
        Some(Action::EditSelected)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::MetadataToggle),
        None
    );

    app.begin_edit_tags();
    if let Some(state) = app.edit_metadata_state_mut() {
        state.all_values.push(("work".to_string(), 1));
        state.filtered.push(0);
    }
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::MetadataToggle),
        Some(Action::MetadataToggle)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::MetadataSave),
        Some(Action::MetadataSave)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::CancelOverlay),
        Some(Action::CancelOverlay)
    );

    // Location hints route to their identically-named actions.
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::LocationResolve),
        Some(Action::LocationResolve)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::LocationSelectRow),
        Some(Action::LocationSelectRow)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::LocationSave),
        Some(Action::LocationSave)
    );
}

#[test]
fn enter_in_metadata_input_saves_when_input_is_empty() {
    let mut app = app_with_entries(1);
    app.begin_edit_tags();
    let state = app.edit_metadata_state_mut().unwrap();
    state.focus = EditMetadataFocus::Input;
    state.input.clear();

    assert_eq!(
        keyboard::key_to_action(
            &app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            true
        ),
        Some(Action::MetadataSave)
    );

    app.edit_metadata_state_mut().unwrap().input = "rust".into();
    assert_eq!(
        keyboard::key_to_action(
            &app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            true
        ),
        Some(Action::MetadataAddFromInput)
    );
}

#[test]
fn arrows_in_metadata_input_move_the_caret_for_mid_string_edits() {
    let mut app = app_with_entries(1);
    app.begin_edit_tags();
    let state = app.edit_metadata_state_mut().unwrap();
    state.focus = EditMetadataFocus::Input;
    state.input = "rst".into();

    // Left in the focused input routes to the field like any editing key...
    assert_eq!(
        keyboard::key_to_action(
            &app,
            KeyEvent::new(KeyCode::Left, KeyModifiers::empty()),
            true
        ),
        Some(Action::InputKey(key(KeyCode::Left)))
    );

    // ...which resolves to this dialog's input and edits at the caret.
    let input = app.focused_text_input_mut().unwrap();
    input.input(key(KeyCode::Left));
    input.input(key(KeyCode::Left));
    input.input(key(KeyCode::Char('u')));
    assert_eq!(
        app.edit_metadata_state().unwrap().input.as_str(),
        "rust",
        "insert lands at the caret, not the end"
    );
}

#[test]
fn wide_journal_click_selects_journal_and_keeps_journal_focus() {
    let mut app = app_with_journals(&["alpha", "beta"]);
    app.nav.focus = Focus::Journals;
    app.nav.selected_entry_index = Some(3);
    app.nav.scroll.entry_view = 10;
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
    // Selecting a journal clears the entry selection so the insights column shows
    // instead of an entry preview.
    assert_eq!(app.nav.selected_entry_index, None);
    assert!(app.show_journal_insights_preview());
    assert_eq!(app.nav.scroll.entry_view, 0);
    assert_eq!(app.nav.focus, Focus::Journals);
}

#[test]
fn compact_journal_click_moves_to_entries() {
    let mut app = app_with_journals(&["work"]);
    app.nav.focus = Focus::Journals;
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
    assert_eq!(app.nav.focus, Focus::Entries);
}

#[test]
fn journal_panel_click_without_row_focuses_journals_without_changing_selection() {
    let mut app = app_with_journals(&["alpha"]);
    app.nav.focus = Focus::Entries;
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
    assert_eq!(app.nav.focus, Focus::Journals);
}

#[test]
fn wheel_over_journals_scrolls_without_changing_selection() {
    let mut app = app_with_journals(&["a", "b", "c", "d", "e", "f", "g"]);
    app.nav.focus = Focus::Entries;
    let layout = render::tui_layout(Rect::new(0, 0, 130, 8), &app);
    let journals = layout.journals.unwrap().content;

    mouse_in_area(
        &mut app,
        mouse(MouseEventKind::ScrollDown, journals.x, journals.y),
        130,
        8,
    );

    assert_eq!(app.selected_journal_index(), 0);
    // Pixel-row lists scroll two rows per notch (their items are several rows tall).
    assert_eq!(app.nav.journal_list.offset(), 2);
    assert_eq!(app.nav.focus, Focus::Entries);
}

#[test]
fn wheel_over_entries_scrolls_without_changing_selection() {
    let mut app = app_with_entries(8);
    app.nav.focus = Focus::Journals;
    let layout = render::tui_layout(Rect::new(0, 0, 90, 8), &app);
    let entries = layout.entries.unwrap().panel.content;

    mouse_in_area(
        &mut app,
        mouse(MouseEventKind::ScrollDown, entries.x, entries.y),
        90,
        8,
    );

    assert_eq!(app.nav.selected_entry_index, Some(0));
    // Pixel-row lists scroll two rows per notch (their items are several rows tall).
    assert_eq!(app.nav.entry_list.offset(), 2);
    assert_eq!(app.nav.focus, Focus::Journals);
}

#[test]
fn entry_click_selects_row_without_opening_viewer_when_entry_view_is_visible() {
    let mut app = app_with_entries(2);
    app.nav.focus = Focus::Journals;
    let layout = render::tui_layout(Rect::new(0, 0, 130, 12), &app);
    let geo = layout.entries.unwrap();
    let entries = geo.panel.content;
    let rows = render::entry_row_metadata(&app, geo.text_width);
    let y_off: u16 = rows
        .iter()
        .take_while(|row| row.item_index != Some(1))
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

    assert_eq!(app.nav.focus, Focus::Entries);
    assert_eq!(app.nav.selected_entry_index, Some(1));
}

#[test]
fn entry_panel_month_divider_click_deselects_to_journal_insights() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::EntryView;
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

    assert_eq!(app.nav.focus, Focus::Entries);
    assert_eq!(app.nav.selected_entry_index, None);
}

#[test]
fn entry_panel_empty_space_click_deselects_to_journal_insights() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::EntryView;
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

    assert_eq!(app.nav.focus, Focus::Entries);
    assert_eq!(app.nav.selected_entry_index, None);
}

#[test]
fn wheel_over_entry_view_scrolls_entry_view_only() {
    let mut app = app_with_entries(6);
    app.nav.focus = Focus::Entries;
    let layout = render::tui_layout(Rect::new(0, 0, 120, 20), &app);
    let entry_view = layout.entry_view.unwrap().content;

    mouse_in_area(
        &mut app,
        mouse(MouseEventKind::ScrollDown, entry_view.x, entry_view.y),
        120,
        20,
    );

    assert_eq!(app.nav.scroll.entry_view, 1);
    assert_eq!(app.nav.entry_list.offset(), 0);
    assert_eq!(app.nav.selected_entry_index, Some(0));
    // Scrolling moves the content under the cursor but leaves the active pane alone.
    assert_eq!(app.nav.focus, Focus::Entries);
}

#[test]
fn expanded_entry_wheel_scrolls_and_clicks_do_not_close() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();

    mouse_in_area(&mut app, mouse(MouseEventKind::ScrollDown, 1, 1), 80, 20);
    assert_eq!(app.nav.scroll.entry_view, 1);

    mouse_in_area(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
        80,
        20,
    );
    assert_eq!(app.nav.focus, Focus::EntryView);
}

#[test]
fn multi_col_fullscreen_body_click_does_not_collapse() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();
    app.nav.entry_view_fullscreen = true;

    // A click inside the full-screen body (not on a metadata chip) must leave the
    // viewer expanded rather than collapsing it back to the pane.
    mouse_in_area(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), 5, 10),
        130,
        20,
    );

    assert_eq!(app.nav.focus, Focus::EntryView);
    assert!(app.nav.entry_view_fullscreen);
}

#[test]
fn metadata_refresh_restores_expanded_entry_view_and_scroll() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();
    app.nav.scroll.entry_view = 7;

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

    assert_eq!(app.nav.focus, Focus::EntryView);
    assert_eq!(app.nav.scroll.entry_view, 7);
    assert_eq!(app.selected_entry_tags(), vec!["work".to_string()]);
    assert!(!app.has_overlay());
}

#[test]
fn confirmed_delete_from_expanded_entry_closes_viewer() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();
    app.nav.scroll.entry_view = 5;
    app.begin_confirm_delete();

    assert_eq!(app.nav.focus, Focus::EntryView);

    confirm_delete(&mut app).unwrap();

    assert_eq!(app.nav.focus, Focus::Entries);
    assert_eq!(app.nav.scroll.entry_view, 0);
    assert_eq!(app.current_entry_list_len(), 0);
    assert!(!app.has_overlay());
}

#[test]
fn search_from_entry_view_resets_focus_and_scroll() {
    let mut app = app_with_entries(1);
    view_selected(&mut app).unwrap();
    app.nav.scroll.entry_view = 5;

    app.begin_search();

    assert_eq!(app.nav.focus, Focus::Entries);
    assert_eq!(app.nav.mode, crate::tui::app::Mode::Search);
    assert_eq!(app.nav.scroll.entry_view, 0);
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

    let config = Config::new(root.clone());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    view_selected(&mut app).unwrap();
    app.nav.scroll.entry_view = 9;

    let store = JournalStore::for_config(&root.join("config.toml"), &root).unwrap();
    let created = store
        .create_entry(
            journal_storage::EntryDraft::new(
                "work",
                "# Created\nBody\n",
                &journal_core::Metadata::default(),
            ),
            journal_storage::EntryAssetOptions::default(),
        )
        .unwrap()
        .path;
    app.refresh().unwrap();
    let created_id = journal_storage::entry_id(&created).unwrap();
    assert!(app.select_entry_by_id(&created_id, true));
    app.nav.focus = Focus::EntryView;

    assert_eq!(app.nav.focus, Focus::EntryView);
    assert_eq!(app.nav.scroll.entry_view, 0);
    assert_eq!(app.selected_entry_target().unwrap().path, created);
}

#[test]
fn wheel_over_tag_dialog_list_scrolls_without_selection_or_toggle_change() {
    let mut app = app_with_entries(1);
    app.begin_edit_tags();
    set_tag_dialog_items(&mut app, 20);
    let layout = render::metadata_dialog_layout(Rect::new(0, 0, 120, 20), 20);

    mouse_in_area(
        &mut app,
        mouse(MouseEventKind::ScrollDown, layout.list.x, layout.list.y),
        120,
        20,
    );

    let state = app.edit_metadata_state().unwrap();
    assert_eq!(state.offset(), 1);
    assert_eq!(state.selected_index(), Some(0));
    assert!(state.selected.is_empty());
}

#[test]
fn click_on_tag_dialog_row_selects_and_toggles_it() {
    let mut app = app_with_entries(1);
    app.begin_edit_tags();
    set_tag_dialog_items(&mut app, 5);
    let layout = render::metadata_dialog_layout(Rect::new(0, 0, 120, 20), 5);

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

    let state = app.edit_metadata_state().unwrap();
    assert_eq!(state.selected_index(), Some(2));
    assert_eq!(state.selected, vec!["tag-02"]);
}

#[test]
fn click_on_tag_dialog_placeholder_row_does_not_toggle() {
    let mut app = app_with_entries(1);
    app.begin_edit_tags();
    let state = app.edit_metadata_state_mut().unwrap();
    state.all_values = vec![("work".to_string(), 1)];
    state.filtered.clear();
    state.input = "missing".into();
    state.normalize_list_state();
    let layout = render::metadata_dialog_layout(Rect::new(0, 0, 120, 12), 0);

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

    let state = app.edit_metadata_state().unwrap();
    assert_eq!(state.selected_index(), None);
    assert!(state.selected.is_empty());
}

#[test]
fn click_on_tag_input_row_switches_focus_to_input() {
    let mut app = app_with_entries(1);
    app.begin_edit_tags();
    set_tag_dialog_items(&mut app, 3);
    let layout = render::metadata_dialog_layout(Rect::new(0, 0, 120, 16), 3);

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

    assert_eq!(
        app.edit_metadata_state().unwrap().focus,
        EditMetadataFocus::Input
    );
}

#[test]
fn click_on_feeling_dialog_header_expands_then_feeling_toggles() {
    let mut app = app_with_entries(1);
    app.begin_edit_feelings();

    let feelings_layout = |app: &App| {
        let state = app.edit_feeling_state().unwrap();
        let all_len = state.item_count();
        let selected_lines = render::feelings_selected_line_count(&state.selected);
        render::feelings_dialog_layout(Rect::new(0, 0, 120, 20), all_len, selected_lines)
    };

    // Clicking the first (header) row folds that group open.
    let layout = feelings_layout(&app);
    mouse_in_area(
        &mut app,
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            layout.list.x,
            layout.list.y,
        ),
        120,
        20,
    );
    assert!(app.edit_feeling_state().unwrap().expanded[0]);

    // The first feeling now sits directly below the header; clicking it selects it.
    let layout = feelings_layout(&app);
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
    let FeelingRow::Feeling { group, feeling } = state.visible_rows()[1] else {
        panic!("row 1 should be a feeling");
    };
    let word = state.groups[group].feelings[feeling].name;
    assert_eq!(state.selected, vec![word.to_string()]);
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
    assert_eq!(app.nav.entry_list.offset(), 1);
    assert!(app.scrollbar.active.is_none());
    assert_eq!(app.nav.focus, Focus::Entries);

    // The up arrow steps back.
    mouse_in_area(&mut app, mouse(down(), bar.x, up_arrow), 120, 20);
    assert_eq!(app.nav.entry_list.offset(), 0);
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
    *app.nav.entry_list.offset_mut() = max / 2;
    let before = app.nav.entry_list.offset();

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
    assert_eq!(app.nav.entry_list.offset(), before);
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
        app.nav.entry_list.offset() > max / 2,
        "expected a large jump, got {}",
        app.nav.entry_list.offset()
    );

    // Drag to the top, cursor drifted off the bar column → scroll to 0.
    mouse_in_area(&mut app, mouse(drag(), 0, top_track), 120, 20);
    assert_eq!(app.nav.entry_list.offset(), 0);

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
    // The journal list uses the same pixel-row model as entries: each box is
    // JOURNAL_BOX_HEIGHT tall, so the total content height is journals × that.
    let list_area = render::journal_list_rect(journals.content);
    let total_height = app.library.journals.len() * render::JOURNAL_BOX_HEIGHT as usize;
    let max = total_height.saturating_sub(list_area.height as usize);
    assert!(max > 0, "journals list should overflow so a bar is drawn");

    // Press the bottom track row → thumb jumps down.
    mouse_in_area(
        &mut app,
        mouse(down(), bar.x, bar.y + bar.height - 2),
        120,
        20,
    );
    assert_eq!(app.scrollbar.active, Some(ScrollbarDrag::Journals));
    assert!(app.nav.journal_list.offset() > 0);
}

// ── Scroll-burst coalescing ────────────────────────────────────────────────

fn wheel_event(kind: MouseEventKind) -> Event {
    Event::Mouse(mouse(kind, 0, 0))
}

#[test]
fn fold_leading_wheel_nets_opposing_scrolls() {
    let up = wheel_event(MouseEventKind::ScrollUp);
    let down = wheel_event(MouseEventKind::ScrollDown);
    // Five up + two down → net -3, all seven consumed.
    let events = vec![
        up.clone(),
        up.clone(),
        up.clone(),
        up.clone(),
        up,
        down.clone(),
        down,
    ];
    assert_eq!(fold_leading_wheel(&events), (-3, 7));
}

#[test]
fn fold_leading_wheel_stops_at_first_non_wheel() {
    let down = wheel_event(MouseEventKind::ScrollDown);
    let key = Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::empty()));
    // Only the leading wheel run is folded; the key stays for later handling.
    let events = vec![
        down.clone(),
        down,
        key,
        wheel_event(MouseEventKind::ScrollUp),
    ];
    assert_eq!(fold_leading_wheel(&events), (2, 2));
}

#[test]
fn fold_leading_wheel_edge_cases() {
    assert_eq!(fold_leading_wheel(&[]), (0, 0));
    let single = vec![wheel_event(MouseEventKind::ScrollUp)];
    assert_eq!(fold_leading_wheel(&single), (-1, 1));
    // A leading non-wheel event consumes nothing.
    let click = vec![Event::Mouse(mouse(down(), 0, 0))];
    assert_eq!(fold_leading_wheel(&click), (0, 0));
}

// ── Settings menu / theme picker routing ─────────────────────────────────────

#[test]
fn comma_opens_settings_in_browse_but_not_over_dialogs() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char(',')), true),
        Some(Action::OpenSettingsMenu)
    );

    // With a dialog open the key belongs to that overlay, not settings.
    app.begin_edit_tags();
    assert_ne!(
        keyboard::key_to_action(&app, key(KeyCode::Char(',')), true),
        Some(Action::OpenSettingsMenu)
    );
}

#[test]
fn settings_menu_routes_enter_and_t_to_the_theme_picker() {
    let mut app = app_with_journals(&["work"]);
    app.open_settings_menu();

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::OpenThemePicker)
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('t')), true),
        Some(Action::OpenThemePicker)
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Esc), true),
        Some(Action::CancelOverlay)
    );
}

#[test]
fn theme_picker_keys_route_to_dedicated_actions() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Up), true),
        Some(Action::ThemePickerMoveUp)
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Down), true),
        Some(Action::ThemePickerMoveDown)
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::ThemePickerConfirm)
    );
    // Esc reverts through the dedicated cancel, not the generic overlay close.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Esc), true),
        Some(Action::ThemePickerCancel)
    );

    // The picker's hint chips route to the same actions.
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::ThemePickerApply),
        Some(Action::ThemePickerConfirm)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::ThemePickerRevert),
        Some(Action::ThemePickerCancel)
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::OpenSettings),
        Some(Action::OpenSettingsMenu)
    );
}

// ── Hover ─────────────────────────────────────────────────────────────────────

#[test]
fn hover_tracks_journal_rows_without_moving_selection() {
    let mut app = app_with_journals(&["work", "zeta"]);
    let area = Rect::new(0, 0, 120, 20);
    let journals = render::tui_layout(area, &app)
        .journals
        .expect("journals panel");
    let list = render::journal_list_rect(journals.content);
    let selected_before = app.nav.journal_list.selected();

    // The middle line of the second journal's row.
    let row = list.y + render::journal_row_height() + 1;
    assert!(mouse::update_hover(&mut app, list.x + 2, row, area));
    assert_eq!(app.hover, HoverTarget::Journal(1));
    assert_eq!(
        app.nav.journal_list.selected(),
        selected_before,
        "hover must never move the journal selection"
    );

    // Motion within the same row doesn't ask for a repaint.
    assert!(!mouse::update_hover(&mut app, list.x + 3, row, area));

    // Any key event clears the glow — the keyboard half of the input mode.
    assert!(app.clear_hover());
    assert_eq!(app.hover, HoverTarget::None);
}

#[test]
fn hover_finds_footer_hints() {
    let mut app = app_with_journals(&["work"]);
    let area = Rect::new(0, 0, 120, 20);
    let footer = render::tui_layout(area, &app).footer;
    let hovered = (footer.x..footer.x + footer.width).any(|col| {
        mouse::update_hover(&mut app, col, footer.y, area);
        matches!(app.hover, HoverTarget::FooterHint(_))
    });
    assert!(hovered, "no footer hint hoverable on the browse footer");
}

#[test]
fn hover_tracks_insights_tabs_without_switching_tabs() {
    let mut app = app_with_entries(1);
    app.nav.selected_entry_index = None;
    app.nav.insights_tab = InsightsTab::Overview;
    let area = Rect::new(0, 0, 140, 20);
    let insights = render::tui_layout(area, &app)
        .insights
        .expect("insights panel");
    let col = (insights.area.x..insights.area.x + insights.area.width)
        .find(|col| {
            render::insights_tab_at(insights.area, *col, insights.area.y)
                == Some(InsightsTab::Writing)
        })
        .expect("writing tab");

    assert!(mouse::update_hover(&mut app, col, insights.area.y, area));
    assert_eq!(app.hover, HoverTarget::InsightsTab(InsightsTab::Writing));
    assert_eq!(app.nav.insights_tab, InsightsTab::Overview);
}

#[test]
fn theme_picker_hover_moves_selection_for_live_preview() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    let area = Rect::new(0, 0, 90, 30);
    let state = app.theme_picker_state().expect("picker open");
    let len = state.entries.len();
    assert!(len > 1, "picker should list the bundled themes");
    let initial = state.selected_index();
    let offset = state.offset();
    let target = if initial == Some(offset) {
        offset + 1
    } else {
        offset
    };
    let layout = render::theme_picker_layout(area, len);

    let row = layout.list.y + (target - offset) as u16;
    assert!(mouse::update_hover(&mut app, layout.list.x + 1, row, area));
    assert_eq!(app.hover, HoverTarget::ThemePickerRow(target));
    // Overlay menus follow the cursor: the hovered row becomes the selection,
    // which live-previews the theme (same path as the arrow keys).
    assert_eq!(
        app.theme_picker_state().unwrap().selected_index(),
        Some(target)
    );
}

#[test]
fn settings_menu_hover_targets_its_rows() {
    let mut app = app_with_journals(&["work"]);
    app.open_settings_menu();
    let area = Rect::new(0, 0, 64, 20);

    // Find the theme row through the same hit-test the click path uses.
    let point = (0..area.height)
        .flat_map(|row| (0..area.width).map(move |col| (col, row)))
        .find(|(col, row)| render::settings_menu_row_at_point(area, *col, *row) == Some(0))
        .expect("settings menu has a hoverable row");
    assert!(mouse::update_hover(&mut app, point.0, point.1, area));
    assert_eq!(app.hover, HoverTarget::DialogRow(0));
}

// ── Toast interaction ─────────────────────────────────────────────────────────

#[test]
fn clicking_a_toast_dismisses_it() {
    let mut app = app_with_journals(&["work"]);
    app.toast(crate::tui::state::ToastVariant::Info, "First");
    app.toast(crate::tui::state::ToastVariant::Error, "Second");
    let area = Rect::new(0, 0, 120, 30);
    let rects = render::toast_rects(&app, area);
    assert_eq!(rects.len(), 2);

    // Click the second toast: only it disappears.
    let target = rects[1];
    mouse_in_area(&mut app, mouse(down(), target.x + 1, target.y + 1), 120, 30);
    let remaining: Vec<_> = app
        .toasts
        .items()
        .iter()
        .map(|toast| toast.message.clone())
        .collect();
    assert_eq!(remaining, vec!["First".to_string()]);

    // A click outside any toast is not swallowed by the dismiss probe.
    mouse_in_area(&mut app, mouse(down(), 0, area.height - 1), 120, 30);
    assert_eq!(app.toasts.items().len(), 1);
}

#[test]
fn hovering_a_toast_targets_it_over_everything() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    app.toast(crate::tui::state::ToastVariant::Info, "Saved");
    let area = Rect::new(0, 0, 120, 30);
    let rect = render::toast_rects(&app, area)[0];

    assert!(mouse::update_hover(&mut app, rect.x + 1, rect.y + 1, area));
    // Even with the picker open, the topmost toast wins the probe.
    assert_eq!(app.hover, HoverTarget::Toast(0));
}

#[test]
fn dialog_list_hover_targets_rows_without_selecting() {
    let mut app = app_with_entries(1);
    app.begin_edit_tags();
    set_tag_dialog_items(&mut app, 5);
    let area = Rect::new(0, 0, 120, 20);
    let layout = render::metadata_dialog_layout(area, 5);

    // The third row: hover targets it, but selection and toggles stay put.
    assert!(mouse::update_hover(
        &mut app,
        layout.list.x,
        layout.list.y + 2,
        area
    ));
    assert_eq!(app.hover, HoverTarget::DialogRow(2));
    let state = app.edit_metadata_state().unwrap();
    assert_eq!(state.selected_index(), Some(0));
    assert!(state.selected.is_empty());
}

#[test]
fn confirm_delete_hover_targets_the_buttons() {
    let mut app = app_with_entries(1);
    let ctx = crate::tui::state::DeleteContext::Entry { has_body: true };
    app.overlay =
        crate::tui::state::Overlay::ConfirmDelete(crate::tui::state::DeleteContext::Entry {
            has_body: true,
        });
    let area = Rect::new(0, 0, 120, 20);
    let inner = render::confirm_delete_inner(area, &ctx);

    // Probe every cell of the buttons row until each button is found.
    let mut saw = (false, false);
    for col in inner.x..inner.x + inner.width {
        for row in inner.y..inner.y + inner.height {
            mouse::update_hover(&mut app, col, row, area);
            match app.hover {
                HoverTarget::ConfirmButton(true) => saw.0 = true,
                HoverTarget::ConfirmButton(false) => saw.1 = true,
                _ => {}
            }
        }
    }
    assert!(saw.0 && saw.1, "both confirm buttons hoverable: {saw:?}");
}

#[test]
fn theme_picker_cycles_chrome_and_cancel_restores_it() {
    use crate::tui::theme::{ChromeStyle, chrome_override};
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    assert_eq!(chrome_override(), None);

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('b')), true),
        Some(Action::ThemePickerCycleChrome)
    );

    // auto → flat → bordered → auto, previewing live.
    app.theme_picker_cycle_chrome();
    assert_eq!(chrome_override(), Some(ChromeStyle::Flat));
    app.theme_picker_cycle_chrome();
    assert_eq!(chrome_override(), Some(ChromeStyle::Bordered));

    // Cancel restores the override from open time along with the theme.
    app.theme_picker_cancel();
    assert_eq!(chrome_override(), None);
}

#[test]
fn theme_picker_confirm_persists_the_chrome_override() {
    use crate::tui::theme::ChromeStyle;
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    app.theme_picker_cycle_chrome();
    app.theme_picker_confirm();
    assert_eq!(app.config.ui.chrome, crate::config::ChromeMode::Flat);
    assert_eq!(
        crate::tui::theme::chrome_override(),
        Some(ChromeStyle::Flat)
    );
    // The saved config round-trips the setting.
    let loaded = crate::config::load_config(&app.config_path).unwrap();
    assert_eq!(loaded.ui.chrome, crate::config::ChromeMode::Flat);
}

#[test]
fn theme_picker_cycles_color_mode_and_cancel_restores_it() {
    use crate::config::ColorMode;
    use crate::tui::theme::{Mode, color_mode, mode};
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    assert_eq!(color_mode(), ColorMode::Auto);

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('m')), true),
        Some(Action::ThemePickerCycleMode)
    );

    // auto → dark → light → auto, previewing live; the resolved mode follows
    // (auto falls back to dark with no detected terminal background).
    app.theme_picker_cycle_mode();
    assert_eq!(color_mode(), ColorMode::Dark);
    app.theme_picker_cycle_mode();
    assert_eq!(color_mode(), ColorMode::Light);
    assert_eq!(mode(), Mode::Light);

    // A mode change re-resolves the picker rows against the new variant.
    let journal_light = app
        .theme_picker_state()
        .and_then(|state| state.entries.iter().find(|entry| entry.name == "journal"))
        .and_then(|entry| entry.theme)
        .expect("bundled journal theme resolves");
    assert_eq!(
        journal_light.bg(),
        ratatui::style::Color::Rgb(0xfc, 0xfc, 0xfc),
        "journal rows must re-resolve to the light variant"
    );

    // Cancel restores the mode from open time along with the theme.
    app.theme_picker_cancel();
    assert_eq!(color_mode(), ColorMode::Auto);
}

#[test]
fn theme_picker_confirm_persists_the_color_mode() {
    use crate::config::ColorMode;
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    app.theme_picker_cycle_mode();
    app.theme_picker_confirm();
    assert_eq!(app.config.ui.color_mode, ColorMode::Dark);
    // The saved config round-trips the setting.
    let loaded = crate::config::load_config(&app.config_path).unwrap();
    assert_eq!(loaded.ui.color_mode, ColorMode::Dark);
}
