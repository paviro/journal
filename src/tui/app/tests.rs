use super::*;
use crate::tui::state::ListNav;
use crate::tui::test_support::{app_with_journals, new_app, new_app_with_state};

fn key_char(ch: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char(ch),
        crossterm::event::KeyModifiers::NONE,
    )
}
use std::fs;
use tempfile::tempdir;

#[test]
fn cache_miss_starts_with_live_journals_while_entries_validate() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config_path = dir.path().join("config/config.toml");
    let config = Config::new(root.clone());
    let store = JournalStore::for_config(&config_path, &root).unwrap();
    store.ensure().unwrap();
    store.create_journal("work").unwrap();
    store
        .create_entry(
            notema_storage::EntryDraft::new("work", "Body", &notema_domain::Metadata::default()),
            notema_storage::EntryAssetOptions::default(),
        )
        .unwrap();

    let (mut app, cached) = App::new_cached(config_path, config, store).unwrap();

    assert!(cached.is_none());
    assert_eq!(app.library.journals.len(), 1);
    assert_eq!(app.library.journals[0].name, "work");
    assert!(app.library.entries.is_empty());
    assert!(!app.library_validated);
    assert_eq!(app.toasts.items().len(), 1);
    assert_eq!(app.toasts.items()[0].message, "Loading journals from disk…");
    assert!(!app.expire_toasts());

    app.finish_initial_library_loading();
    assert!(app.toasts.items().is_empty());

    app.begin_manual_refresh();
    assert_eq!(app.toasts.items()[0].message, "Refreshing from disk…");
    assert!(!app.expire_toasts());
    app.finish_manual_refresh();
    assert!(app.toasts.items().is_empty());
}

#[test]
fn changing_selected_entry_resets_reader_scroll() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# A\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# B\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;
    app.nav.scroll.reader = 20;

    app.move_selection(1);

    assert_eq!(app.nav.scroll.reader, 0);
}

#[test]
fn scrolling_up_past_first_entry_deselects_and_shows_insights() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# A\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# B\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    assert_eq!(app.nav.selected_entry_index, Some(0));

    // Up from the first entry deselects, revealing the journal insights reader.
    app.move_selection(-1);
    assert_eq!(app.nav.selected_entry_index, None);
    assert!(app.show_journal_insights());
    assert!(!app.entries_highlighted());
    assert!(app.selected_entry_target().is_none());

    // Down reselects the first entry.
    app.move_selection(1);
    assert_eq!(app.nav.selected_entry_index, Some(0));
    assert!(!app.show_journal_insights());
}

#[test]
fn focusing_journals_shows_insights_even_with_a_lingering_entry_selection() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# A\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.select_entry_index(0);
    app.nav.focus = Focus::Entries;
    // Focused on the entry, its reader shows and its row is highlighted.
    assert!(!app.show_journal_insights());
    assert!(app.entries_highlighted());

    // Moving focus back to the journal column (e.g. clicking the already-selected
    // journal, or Left from the entry) leaves the selection index untouched, but the
    // right column must revert to insights and the row must lose its highlight — the two
    // never disagree.
    app.nav.focus = Focus::Journals;
    assert_eq!(app.nav.selected_entry_index, Some(0));
    assert!(app.show_journal_insights());
    assert!(!app.entries_highlighted());
}

#[test]
fn focusing_insights_shows_insights_even_with_a_lingering_entry_selection() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# A\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.select_entry_index(0);
    app.nav.focus = Focus::Entries;
    assert!(!app.show_journal_insights());
    assert!(app.entries_highlighted());

    // Clicking the insights column focuses it but leaves the selection index
    // lingering. The insights panel shares the right-hand pane with the entry
    // viewer, so it must show insights and drop the row highlight — not reopen
    // the entry that was just closed.
    app.nav.focus = Focus::Insights;
    assert_eq!(app.nav.selected_entry_index, Some(0));
    assert!(app.show_journal_insights());
    assert!(!app.entries_highlighted());
}

#[test]
fn hidden_journals_launch_focuses_entries_with_insights_reader() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# A\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut state = crate::config::State::default();
    state.ui.show_journals = false;
    let app = new_app_with_state(config, state);

    assert_eq!(app.nav.focus, Focus::Entries);
    assert_eq!(app.nav.selected_entry_index, None);
    assert!(app.show_journal_insights());
}

#[test]
fn selected_reader_title_uses_entry_timestamp() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:23:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");

    let (title, content) = app.selected_reader().unwrap();

    assert_eq!(title, "Wednesday, 1 July 2026, 10:23");
    assert_eq!(content, "# A\nBody\n");
}

#[test]
fn search_reader_title_uses_entry_timestamp() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:23:00+02:00\"\n+++\n\n# A\nneedle\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.begin_search();
    app.search.query = "needle".into();
    app.update_search_results();

    let (title, content) = app.selected_reader().unwrap();

    assert_eq!(title, "Wednesday, 1 July 2026, 10:23");
    assert_eq!(content, "# A\nneedle\n");
}

#[test]
fn journal_focus_does_not_make_entry_targets_actionable() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\ntags = []\n+++\n\n# A\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");

    app.nav.focus = Focus::Journals;
    assert!(!app.can_act_on_selected_entry());

    app.nav.focus = Focus::Entries;
    assert!(app.can_act_on_selected_entry());
}

#[test]
fn compact_width_uses_single_panel_without_inline_reader() {
    assert!(single_panel_is_active(TWO_PANEL_MIN_WIDTH - 1));
    assert!(!inline_reader_is_visible(TWO_PANEL_MIN_WIDTH - 1));
    assert!(!reader_is_available(TWO_PANEL_MIN_WIDTH - 1));
    assert!(reader_is_available(TWO_PANEL_MIN_WIDTH));
}

#[test]
fn inline_reader_uses_minimum_three_column_width() {
    assert!(!inline_reader_is_visible(INLINE_READER_MIN_WIDTH - 1));
    assert!(inline_reader_is_visible(INLINE_READER_MIN_WIDTH));
}

#[test]
fn search_from_journal_focus_is_global() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);
    app.nav.focus = Focus::Journals;

    app.begin_search();

    assert_eq!(app.search.scope, SearchScope::AllJournals);
}

#[test]
fn search_from_entries_focus_is_scoped_to_selected_journal() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("work")).unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;

    app.begin_search();

    assert_eq!(app.search.scope, SearchScope::Journal("work".to_string()));
}

#[test]
fn empty_search_has_no_selected_entry() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);

    app.begin_search();

    assert_eq!(app.nav.selected_entry_index, None);
    assert!(app.selected_entry_target().is_none());
}

#[test]
fn feelings_search_matches_exact_known_label() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\nfeelings = [\"calm\"]\n+++\n\n# A\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\nschema_version = 1\nfeelings = [\"anxious\"]\n+++\n\n# B\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.begin_search();
    app.search.query = "feelings:calm".into();
    app.update_search_results();

    assert_eq!(app.search.hits.len(), 1);
    assert_eq!(app.search.hits[0].title, "A");
}

#[test]
fn starred_search_filters_by_flag() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\nstarred = true\n+++\n\n# Fav\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\nschema_version = 1\n+++\n\n# Plain\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.begin_search();

    app.search.query = "star:true".into();
    app.update_search_results();
    assert_eq!(app.search.hits.len(), 1);
    assert_eq!(app.search.hits[0].title, "Fav");

    app.search.query = "star:false".into();
    app.update_search_results();
    assert_eq!(app.search.hits.len(), 1);
    assert_eq!(app.search.hits[0].title, "Plain");

    // 1/0 are accepted as boolean aliases.
    app.search.query = "star:1".into();
    app.update_search_results();
    assert_eq!(app.search.hits.len(), 1);
    assert_eq!(app.search.hits[0].title, "Fav");

    app.search.query = "star:0".into();
    app.update_search_results();
    assert_eq!(app.search.hits.len(), 1);
    assert_eq!(app.search.hits[0].title, "Plain");

    // An unparseable flag matches nothing.
    app.search.query = "star:maybe".into();
    app.update_search_results();
    assert!(app.search.hits.is_empty());
}

#[test]
fn begin_edit_feelings_uses_fixed_list_and_selected_entry_values() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\nfeelings = [\"calm\", \"excited\"]\n+++\n\n# A\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");

    app.begin_edit_feelings();

    let state = app.edit_feeling_state().unwrap();
    // Groups start collapsed: only headers are visible and the cursor rests on the
    // first one. The entry's stored feelings are preselected regardless.
    let rows = state.visible_rows();
    assert_eq!(rows.len(), FEELING_GROUPS.len());
    assert!(matches!(rows[0], FeelingRow::Header { group: 0 }));
    assert!(matches!(&state.groups[0], g if g.name == "Joy & Delight"));
    assert_eq!(state.list.selected(), Some(0));
    assert_eq!(state.selected, vec!["calm", "excited"]);
}

#[test]
fn location_dialog_seeds_from_editor_draft_not_selected_entry() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\n[location]\nname = \"Home\"\nlatitude = 52.5\nlongitude = 13.4\n+++\n\n# A\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");

    // Without an editor, the dialog seeds from the selected entry.
    app.begin_edit_location();
    let state = app.edit_location_state().unwrap();
    assert_eq!(state.name.as_str(), "Home");
    assert!(!state.query.is_empty());
    app.close_overlay();

    // Composing a new entry: the dialog seeds from the (empty) editor draft,
    // not the entry that happens to still be selected underneath.
    app.open_editor_for_new();
    app.begin_edit_location();
    let state = app.edit_location_state().unwrap();
    assert!(state.name.is_empty());
    assert!(state.query.is_empty());
    assert!(state.resolved.is_none());
}

#[test]
fn toast_deadline_is_none_without_toasts() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let app = new_app(config);

    assert!(app.toast_deadline().is_none());
}

#[test]
fn toast_deadline_is_some_with_an_active_toast() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);

    app.toast(ToastVariant::Success, "Saved");

    assert!(app.toast_deadline().is_some());
}

#[test]
fn expire_toasts_drops_only_expired_ones_and_reports_once() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);
    app.toasts.push_expired(ToastVariant::Info, "Old");
    app.toast(ToastVariant::Success, "Fresh");

    assert!(app.expire_toasts());
    let messages: Vec<&str> = app
        .toasts
        .items()
        .iter()
        .map(|toast| toast.message.as_str())
        .collect();
    assert_eq!(messages, ["Fresh"]);
    assert!(!app.expire_toasts());
}

#[test]
fn next_countdown_step_is_at_most_one_column_of_lifetime() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);
    app.toast(ToastVariant::Success, "Saved");

    // A fresh toast over 40 columns steps roughly every lifetime/40; the wake is
    // scheduled no later than one such column so the shrink never skips a step.
    let step = app.toasts.next_countdown_step(40).unwrap();
    assert!(step <= std::time::Duration::from_millis(5000 / 40));

    // With no columns to draw (terminal too narrow) there is nothing to animate.
    assert!(app.toasts.next_countdown_step(0).is_none());
}

#[test]
fn long_messages_stay_up_longer_than_short_ones() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);

    // A short confirmation sits at the 5s floor.
    app.toast(ToastVariant::Success, "Saved");
    let short = app.toast_deadline().unwrap();
    assert!(short <= std::time::Duration::from_secs(5));

    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);
    // A long error lingers, capped at 10s.
    app.toast(ToastVariant::Error, "e".repeat(200));
    let long = app.toast_deadline().unwrap();
    assert!(long > std::time::Duration::from_secs(5));
    assert!(long <= std::time::Duration::from_secs(10));
}

#[test]
fn next_countdown_step_is_none_for_an_expired_toast() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);
    app.toasts.push_expired(ToastVariant::Info, "Old");

    // Its line is already empty, so there is no further column to schedule.
    assert!(app.toasts.next_countdown_step(40).is_none());
}

#[test]
fn toast_queue_caps_at_the_four_newest() {
    let config = Config::new(tempdir().unwrap().path().to_path_buf());
    let mut app = new_app(config);

    for n in 0..6 {
        app.toast(ToastVariant::Info, format!("toast {n}"));
    }

    let messages: Vec<&str> = app
        .toasts
        .items()
        .iter()
        .map(|toast| toast.message.as_str())
        .collect();
    assert_eq!(messages, ["toast 2", "toast 3", "toast 4", "toast 5"]);
}

#[test]
fn entry_rows_cache_is_reused_until_inputs_change() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nBody\n",
    )
    .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");

    let first = app.entry_rows(30);
    // Same inputs → same cached rows (identity, not just equality).
    assert!(Rc::ptr_eq(&first, &app.entry_rows(30)));
    // Moving the selection does not change the rows, so the cache holds.
    app.move_selection(1);
    assert!(Rc::ptr_eq(&first, &app.entry_rows(30)));
    // A different width rebuilds.
    assert!(!Rc::ptr_eq(&first, &app.entry_rows(20)));
    // Reloading the store rebuilds.
    app.refresh().unwrap();
    assert!(!Rc::ptr_eq(&first, &app.entry_rows(30)));
}

#[test]
fn search_typing_defers_hit_recompute_until_committed() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\nschema_version = 1\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nneedle\n",
    )
    .unwrap();
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.begin_search();

    for ch in "needle".chars() {
        app.search_input_key(key_char(ch));
    }
    // The query echoes immediately, but the whole-corpus scan is deferred.
    assert_eq!(app.search.query.as_str(), "needle");
    assert!(app.search.dirty);
    assert!(app.search.hits.is_empty());

    // Committing (what the event loop does after the debounce) runs the scan.
    app.update_search_results();
    assert!(!app.search.dirty);
    assert_eq!(app.search.hits.len(), 1);
}

fn write_entry(dir: &std::path::Path, name: &str, created: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        format!("+++\nschema_version = 1\n[datetime]\ncreated_at = \"{created}\"\n+++\n\n{body}\n"),
    )
    .unwrap();
    path
}

#[test]
fn refresh_paths_updates_only_the_changed_entry() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    let a = write_entry(
        &entry_dir,
        "a.md",
        "2026-07-01T10:00:00+02:00",
        "# A\nold body",
    );
    write_entry(&entry_dir, "b.md", "2026-07-01T11:00:00+02:00", "# B\nbee");
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    assert_eq!(app.library.entries.len(), 2);

    // Edit a.md on disk, then reload just that path.
    write_entry(
        &entry_dir,
        "a.md",
        "2026-07-01T10:00:00+02:00",
        "# A\nnew body here",
    );
    app.refresh_paths(&[a]).unwrap();

    assert_eq!(app.library.entries.len(), 2);
    let updated = app.library.entry_by_id("a").unwrap();
    assert!(updated.body.contains("new body here"));
    // Precomputed word count is rebuilt from the fresh body on re-read.
    assert_eq!(updated.word_count, updated.body.split_whitespace().count());
    assert!(!updated.search_haystack.is_empty());
    // `entries` stays sorted by path (descending) so `journal_ranges` holds.
    assert!(
        app.library
            .entries
            .windows(2)
            .all(|w| w[0].path > w[1].path)
    );
    assert_eq!(app.selected_entries().len(), 2);
}

#[test]
fn refresh_paths_handles_create_and_delete() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    let a = write_entry(
        &entry_dir,
        "a.md",
        "2026-07-01T10:00:00+02:00",
        "# A\nalpha",
    );
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    assert_eq!(app.library.entries.len(), 1);

    // A newly written file is picked up by its path alone.
    let c = write_entry(&entry_dir, "c.md", "2026-07-01T12:00:00+02:00", "# C\nsea");
    app.refresh_paths(std::slice::from_ref(&c)).unwrap();
    assert_eq!(app.library.entries.len(), 2);
    assert!(app.library.entry_by_id("c").is_some());

    // Deleting the file on disk removes it on the next targeted reload.
    fs::remove_file(&a).unwrap();
    app.refresh_paths(&[a]).unwrap();
    assert_eq!(app.library.entries.len(), 1);
    assert!(app.library.entry_by_id("a").is_none());
    assert_eq!(app.selected_entries().len(), 1);
}

#[test]
fn refresh_paths_falls_back_to_full_reload_for_a_new_journal() {
    let dir = tempdir().unwrap();
    let work = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&work).unwrap();
    write_entry(&work, "a.md", "2026-07-01T10:00:00+02:00", "# A\nalpha");
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");

    // A path under a brand-new journal isn't attributable to a known journal,
    // so the incremental path must fall back to a full reload that also picks
    // up the new journal in the list.
    let personal = dir.path().join("personal").join("2026-07-01");
    fs::create_dir_all(&personal).unwrap();
    let z = write_entry(&personal, "z.md", "2026-07-02T10:00:00+02:00", "# Z\nzed");
    app.refresh_paths(&[z]).unwrap();

    assert!(
        app.library
            .journals
            .iter()
            .any(|journal| journal.name == "personal")
    );
    assert!(app.library.entry_by_id("z").is_some());
}

#[test]
fn entry_body_cache_is_reused_until_entry_or_width_changes() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    write_entry(&entry_dir, "a.md", "2026-07-01T10:00:00+02:00", "# A\nBody");
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    let path = app.selected_entry_target().map(|target| target.path);

    let body = |text| RenderedEntryBody {
        lines: vec![Line::from(text)],
        ..RenderedEntryBody::default()
    };
    let first = app.cached_entry_body(path.as_deref(), 40, || body("x"));
    // Same entry + width → cached rows returned, the builder isn't re-run.
    let same = app.cached_entry_body(path.as_deref(), 40, || body("y"));
    assert!(Rc::ptr_eq(&first, &same));
    // A different width rebuilds.
    let narrower = app.cached_entry_body(path.as_deref(), 20, || body("z"));
    assert!(!Rc::ptr_eq(&first, &narrower));
    // Reloading the store bumps entries_version, invalidating the cache.
    app.refresh().unwrap();
    let after = app.cached_entry_body(path.as_deref(), 40, || body("w"));
    assert!(!Rc::ptr_eq(&first, &after));
}

#[test]
fn search_recompute_keeps_body_and_analytics_caches_but_rebuilds_rows() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    write_entry(&entry_dir, "a.md", "2026-07-01T10:00:00+02:00", "# A\nbody");
    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    let path = app.selected_entry_target().map(|target| target.path);

    // Prime all three caches.
    let body = app.cached_entry_body(path.as_deref(), 40, || RenderedEntryBody {
        lines: vec![Line::from("x")],
        ..RenderedEntryBody::default()
    });
    let analytics = app.cached_analytics().unwrap();
    let rows = app.entry_rows(30);

    // A search recompute changes the hits but not the entries, so it bumps
    // only rows_version.
    app.begin_search();
    for ch in "body".chars() {
        app.search_input_key(key_char(ch));
    }
    app.update_search_results();

    // Body and analytics caches key on entries_version, which is untouched:
    // requerying returns the same Rc (builder skipped).
    let body_after = app.cached_entry_body(path.as_deref(), 40, || RenderedEntryBody {
        lines: vec![Line::from("y")],
        ..RenderedEntryBody::default()
    });
    assert!(Rc::ptr_eq(&body, &body_after));
    let analytics_after = app.cached_analytics().unwrap();
    assert!(Rc::ptr_eq(&analytics, &analytics_after));

    // The row cache keys on rows_version, which the recompute bumped, so it
    // rebuilt.
    let rows_after = app.entry_rows(30);
    assert!(!Rc::ptr_eq(&rows, &rows_after));
}

#[test]
fn metadata_partitioned_excludes_archived_and_isolates_archived_only() {
    let dir = tempdir().unwrap();
    let active_dir = dir.path().join("work").join("2026-07-01");
    let archived_dir = dir.path().join("old.archived").join("2026-07-01");
    fs::create_dir_all(&active_dir).unwrap();
    fs::create_dir_all(&archived_dir).unwrap();
    fs::write(
        active_dir.join("a.md"),
        "+++\nschema_version = 1\ntags = [\"berlin\", \"shared\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\n",
    )
    .unwrap();
    fs::write(
        archived_dir.join("b.md"),
        "+++\nschema_version = 1\ntags = [\"wanderlust\", \"shared\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# B\n",
    )
    .unwrap();

    let app = new_app(Config::new(dir.path().to_path_buf()));
    let (active, archived_only) = app.metadata_partitioned(MetadataKind::Tags);

    let active_tags: Vec<&str> = active.iter().map(|(t, _)| t.as_str()).collect();
    assert!(active_tags.contains(&"berlin"));
    assert!(active_tags.contains(&"shared"));
    // Archived usage doesn't leak into the active list or its counts.
    assert!(!active_tags.contains(&"wanderlust"));

    // Only values living *solely* in archived journals are surfaced; "shared"
    // also appears in the active journal, so it's not archived-only.
    let archived_tags: Vec<&str> = archived_only.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(archived_tags, vec!["wanderlust"]);
}

#[test]
fn archiving_journal_renames_reorders_and_keeps_entries_resolvable() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("personal").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    write_entry(&entry_dir, "a.md", "2026-07-01T10:00:00+02:00", "# A\nbody");
    fs::create_dir_all(dir.path().join("work")).unwrap();

    let mut app = new_app(Config::new(dir.path().to_path_buf()));

    app.store.set_journal_archived("personal", true).unwrap();
    app.refresh().unwrap();

    // The directory was renamed and the journal now sorts after active ones.
    assert!(dir.path().join("personal.archived").is_dir());
    assert!(!dir.path().join("personal").exists());
    let names: Vec<&str> = app
        .library
        .journals
        .iter()
        .map(|j| j.name.as_str())
        .collect();
    assert_eq!(names, vec!["work", "personal.archived"]);

    // Its entry still resolves under the suffixed identity (the critical
    // invariant: the raw name stays the lookup key).
    app.select_journal_by_name("personal.archived");
    let selected = app.selected_journal().unwrap();
    assert!(selected.archived);
    assert_eq!(selected.display_name(), "personal");
    assert_eq!(app.selected_entries().len(), 1);
}

#[test]
fn refresh_preserves_journal_pixel_scroll_offset() {
    // The journal list scrolls in pixels, not item indices. A refresh must clamp
    // only the selection and leave the offset alone; the old index-based normalize
    // treated the pixel offset as an index and snapped it to `len - 1`, jumping
    // the scroll on every reload.
    let mut app = app_with_journals(&["a", "b", "c"]);
    // A pixel offset far above the 3-journal count — an index clamp would shrink it.
    *app.nav.journal_list.offset_mut() = 15;

    app.refresh().unwrap();

    assert_eq!(app.nav.journal_list.offset(), 15);
}

// ── Settings menu / theme picker ──────────────────────────────────────────────

fn journal_theme(name: &str) -> notema_storage::JournalTheme {
    notema_storage::JournalTheme {
        name: name.to_string(),
        color_mode: None,
        chrome: None,
    }
}

#[test]
fn effective_theme_prefers_journal_then_global_and_respects_ignore() {
    let mut app = app_with_journals(&["work"]);
    app.config.ui.theme = "globaltheme".to_string();
    app.select_journal(0);

    // No per-journal theme → the global theme.
    assert_eq!(app.effective_theme_name(), "globaltheme");

    // A per-journal theme wins over the global one.
    app.library.journals[0].theme = Some(journal_theme("journaltheme"));
    assert_eq!(app.effective_theme_name(), "journaltheme");

    // ignore_journal_themes forces the global theme regardless.
    app.config.ui.ignore_journal_themes = true;
    assert_eq!(app.effective_theme_name(), "globaltheme");
}

#[test]
fn effective_selection_falls_back_to_config_per_field() {
    use crate::config::{ChromeMode, ColorMode};
    let mut app = app_with_journals(&["work"]);
    app.config.ui.theme = "globaltheme".to_string();
    app.config.ui.color_mode = ColorMode::Dark;
    app.config.ui.chrome = ChromeMode::Bordered;
    app.select_journal(0);

    // The journal sets a theme and mode but no chrome; an unknown spelling (from
    // a newer device) counts as unset. Both fall back to the config.
    app.library.journals[0].theme = Some(notema_storage::JournalTheme {
        name: "journaltheme".to_string(),
        color_mode: Some("light".to_string()),
        chrome: Some("holographic".to_string()),
    });
    let selection = app.effective_selection();
    assert_eq!(selection.name, "journaltheme");
    assert_eq!(selection.color_mode, ColorMode::Light);
    assert_eq!(selection.chrome, ChromeMode::Bordered);
}

#[test]
fn theme_picker_opens_on_the_active_theme_with_bundled_entries() {
    let mut app = app_with_journals(&["work"]);
    app.config.ui.theme = "eclipse".to_string();

    app.open_theme_picker();

    let state = app.theme_picker_state().expect("picker open");
    // The bundled themes were materialized and listed, sorted by name.
    let names: Vec<&str> = state.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "arcade",
            "blossom",
            "celadon",
            "classic",
            "crt",
            "cyberpunk",
            "deep-space",
            "dungeon",
            "eclipse",
            "eldritch",
            "fjord",
            "gameboy",
            "grove",
            "hal",
            "indigo",
            "journal",
            "lavender",
            "maple",
            "matcha",
            "matrix",
            "rose-pine",
            "synthwave",
            "tokyonight",
            "tron",
            "vaporwave",
            "wasteland",
        ]
    );
    assert!(state.entries.iter().all(|entry| entry.theme.is_some()));
    // Selection seeds on the configured theme.
    assert_eq!(
        state.selected_index(),
        names.iter().position(|n| *n == "eclipse")
    );
    assert_eq!(state.previous_name, "eclipse");
}

#[test]
fn theme_picker_confirm_saves_the_config_and_closes() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    let fjord = app
        .theme_picker_state()
        .unwrap()
        .entries
        .iter()
        .position(|entry| entry.name == "fjord")
        .unwrap();

    app.theme_picker_select(fjord);
    app.theme_picker_confirm();

    assert!(!app.has_overlay());
    assert_eq!(app.config.ui.theme, "fjord");
    // The change was persisted, not just held in memory.
    let saved = crate::config::load_config(&app.config_path).unwrap();
    assert_eq!(saved.ui.theme, "fjord");
    assert!(
        app.toasts
            .items()
            .iter()
            .any(|toast| toast.message == "Global theme set to fjord")
    );
}

#[test]
fn theme_picker_journal_scope_writes_the_sidecar_not_the_global_theme() {
    let mut app = app_with_journals(&["work"]);
    app.config.ui.theme = "blossom".to_string();
    crate::config::save_config(&app.config_path, &app.config).unwrap();
    app.select_journal(0);
    app.open_theme_picker();
    // Switch to this-journal scope, pick a theme, preview a chrome, confirm.
    app.theme_picker_toggle_scope();
    let gameboy = app
        .theme_picker_state()
        .unwrap()
        .entries
        .iter()
        .position(|entry| entry.name == "gameboy")
        .unwrap();
    app.theme_picker_select(gameboy);
    app.theme_picker_cycle_chrome();

    app.theme_picker_confirm();

    // The journal carries the theme with the previewed mode and chrome; the
    // global settings are untouched, in memory and on disk.
    let expected = notema_storage::JournalTheme {
        name: "gameboy".to_string(),
        color_mode: Some("auto".to_string()),
        chrome: Some("flat".to_string()),
    };
    assert_eq!(app.library.journals[0].theme, Some(expected.clone()));
    assert_eq!(app.effective_theme_name(), "gameboy");
    assert_eq!(app.config.ui.theme, "blossom");
    assert_eq!(app.config.ui.chrome, crate::config::ChromeMode::Default);
    let saved = crate::config::load_config(&app.config_path).unwrap();
    assert_eq!(saved.ui.theme, "blossom");
    assert_eq!(saved.ui.chrome, crate::config::ChromeMode::Default);
    // Persisted to the sidecar, reloadable.
    let reloaded = app.store.list_journals().unwrap();
    let work = reloaded.iter().find(|j| j.name == "work").unwrap();
    assert_eq!(work.theme, Some(expected));
}

#[test]
fn theme_picker_global_scope_clears_a_journal_override() {
    let mut app = app_with_journals(&["work"]);
    app.select_journal(0);
    app.store
        .set_journal_theme("work", Some(&journal_theme("gameboy")))
        .unwrap();
    app.library.journals[0].theme = Some(journal_theme("gameboy"));

    app.open_theme_picker();
    // Opens in Journal scope (the journal has a theme); switch to Global and save.
    app.theme_picker_toggle_scope();
    let fjord = app
        .theme_picker_state()
        .unwrap()
        .entries
        .iter()
        .position(|entry| entry.name == "fjord")
        .unwrap();
    app.theme_picker_select(fjord);
    app.theme_picker_confirm();

    assert_eq!(app.config.ui.theme, "fjord");
    // The journal's override was removed; it now follows global.
    assert_eq!(app.library.journals[0].theme, None);
    let reloaded = app.store.list_journals().unwrap();
    let work = reloaded.iter().find(|j| j.name == "work").unwrap();
    assert_eq!(work.theme, None);
    assert_eq!(app.effective_theme_name(), "fjord");
}

#[test]
fn theme_picker_toggle_scope_moves_the_highlight_to_that_scopes_theme() {
    let mut app = app_with_journals(&["work"]);
    app.config.ui.theme = "blossom".to_string();
    app.select_journal(0);
    app.store
        .set_journal_theme("work", Some(&journal_theme("gameboy")))
        .unwrap();
    app.library.journals[0].theme = Some(journal_theme("gameboy"));

    app.open_theme_picker();
    // Opens in Journal scope, highlighting the journal's own theme.
    assert_eq!(
        app.theme_picker_state()
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "gameboy"
    );
    // Toggling to Global moves the highlight to the global default, not just the
    // preview, so the selected row matches the applied theme.
    app.theme_picker_toggle_scope();
    assert_eq!(
        app.theme_picker_state()
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "blossom"
    );
    // And back again.
    app.theme_picker_toggle_scope();
    assert_eq!(
        app.theme_picker_state()
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "gameboy"
    );
}

#[test]
fn theme_picker_toggle_scope_previews_that_scopes_mode_and_chrome() {
    use crate::config::{ChromeMode, ColorMode};
    let mut app = app_with_journals(&["work"]);
    app.config.ui.theme = "blossom".to_string();
    app.config.ui.color_mode = ColorMode::Light;
    app.config.ui.chrome = ChromeMode::Bordered;
    app.select_journal(0);
    app.library.journals[0].theme = Some(notema_storage::JournalTheme {
        name: "gameboy".to_string(),
        color_mode: Some("dark".to_string()),
        chrome: Some("flat".to_string()),
    });

    // Opens in Journal scope; toggling to Global snaps the previewed mode and
    // chrome to the config values, and back to the journal's own.
    app.open_theme_picker();
    app.theme_picker_toggle_scope();
    assert_eq!(crate::tui::theme::color_mode(), ColorMode::Light);
    assert_eq!(
        crate::tui::theme::chrome_override(),
        Some(crate::tui::theme::ChromeStyle::Bordered)
    );
    app.theme_picker_toggle_scope();
    assert_eq!(crate::tui::theme::color_mode(), ColorMode::Dark);
    assert_eq!(
        crate::tui::theme::chrome_override(),
        Some(crate::tui::theme::ChromeStyle::Flat)
    );
}

#[test]
fn switching_journals_switches_the_effective_theme() {
    let mut app = app_with_journals(&["personal", "work"]);
    app.config.ui.theme = "globaltheme".to_string();
    let work = app
        .library
        .journals
        .iter()
        .position(|j| j.name == "work")
        .unwrap();
    app.library.journals[work].theme = Some(journal_theme("worktheme"));

    app.select_journal_by_name("work");
    assert_eq!(app.effective_theme_name(), "worktheme");
    app.select_journal_by_name("personal");
    assert_eq!(app.effective_theme_name(), "globaltheme");
}

#[test]
fn all_journals_search_uses_the_global_theme_and_exit_restores() {
    let mut app = app_with_journals(&["work"]);
    app.config.ui.theme = "globaltheme".to_string();
    app.select_journal(0);
    app.library.journals[0].theme = Some(journal_theme("journaltheme"));
    assert_eq!(app.effective_theme_name(), "journaltheme");

    // From the journal column, search covers all journals: cross-journal hits
    // shouldn't re-theme per hit, so the global theme applies.
    app.nav.focus = Focus::Journals;
    app.begin_search();
    assert_eq!(app.effective_theme_name(), "globaltheme");
    app.exit_search();
    assert_eq!(app.effective_theme_name(), "journaltheme");
}

#[test]
fn journal_scoped_search_keeps_that_journals_theme() {
    let mut app = app_with_journals(&["work"]);
    app.config.ui.theme = "globaltheme".to_string();
    app.select_journal(0);
    app.library.journals[0].theme = Some(journal_theme("journaltheme"));

    app.nav.focus = Focus::Entries;
    app.begin_search();
    assert_eq!(app.search.scope, SearchScope::Journal("work".to_string()));
    assert_eq!(app.effective_theme_name(), "journaltheme");
}

#[test]
fn compose_uses_the_target_journals_theme() {
    let mut app = app_with_journals(&["personal", "work"]);
    app.config.ui.theme = "globaltheme".to_string();
    let work = app
        .library
        .journals
        .iter()
        .position(|j| j.name == "work")
        .unwrap();
    app.library.journals[work].theme = Some(journal_theme("worktheme"));
    // A different journal is selected (as when state restores the last one).
    app.select_journal_by_name("personal");
    assert_eq!(app.effective_theme_name(), "globaltheme");

    app.begin_compose("work".to_string(), notema_domain::Metadata::default());
    assert_eq!(app.effective_theme_name(), "worktheme");
}

#[test]
fn theme_picker_cancel_reverts_the_preview_and_leaves_config_untouched() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    let previous = app.theme_picker_state().unwrap().previous;
    let eclipse = app
        .theme_picker_state()
        .unwrap()
        .entries
        .iter()
        .position(|entry| entry.name == "eclipse")
        .unwrap();

    // Moving the selection shows the entry immediately…
    app.theme_picker_select(eclipse);
    assert_ne!(crate::tui::theme::theme(), previous);

    // …and Esc restores the open-time theme without touching the config.
    app.theme_picker_cancel();

    assert!(!app.has_overlay());
    assert_eq!(crate::tui::theme::theme(), previous);
    assert_eq!(app.config.ui.theme, crate::tui::theme::DEFAULT_THEME);
    assert!(
        !app.config_path.exists(),
        "cancel must not write the config"
    );
}

#[test]
fn theme_picker_confirm_on_a_broken_theme_toasts_and_stays_open() {
    let mut app = app_with_journals(&["work"]);
    let themes = crate::tui::theme::themes_dir(&app.config_path);
    fs::create_dir_all(&themes).unwrap();
    fs::write(themes.join("busted.toml"), "surfaces = 12\n").unwrap();
    app.open_theme_picker();
    let busted = app
        .theme_picker_state()
        .unwrap()
        .entries
        .iter()
        .position(|entry| entry.name == "busted")
        .unwrap();
    assert!(
        app.theme_picker_state().unwrap().entries[busted]
            .theme
            .is_none(),
        "broken file should fail to parse"
    );

    let before = crate::tui::theme::theme();
    app.theme_picker_select(busted);
    // A broken row never shows an entry.
    assert_eq!(crate::tui::theme::theme(), before);

    app.theme_picker_confirm();

    assert!(app.theme_picker_state().is_some(), "picker stays open");
    assert_eq!(app.config.ui.theme, crate::tui::theme::DEFAULT_THEME);
    assert!(
        app.toasts
            .items()
            .iter()
            .any(|toast| toast.message.contains("broken"))
    );
}
