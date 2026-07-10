use super::*;
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
fn changing_selected_entry_resets_entry_view_scroll() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();
    fs::write(entry_dir.join("b.md"), "+++\ntags = []\n+++\n\n# B\n").unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.nav.focus = Focus::Entries;
    app.nav.scroll.entry_view = 20;

    app.move_selection(1);

    assert_eq!(app.nav.scroll.entry_view, 0);
}

#[test]
fn scrolling_up_past_first_entry_deselects_and_shows_insights() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();
    fs::write(entry_dir.join("b.md"), "+++\ntags = []\n+++\n\n# B\n").unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    assert_eq!(app.nav.selected_entry_index, Some(0));

    // Up from the first entry deselects, revealing the journal insights preview.
    app.move_selection(-1);
    assert_eq!(app.nav.selected_entry_index, None);
    assert!(app.show_journal_insights_preview());
    assert!(!app.entries_highlighted());
    assert!(app.selected_entry_target().is_none());

    // Down reselects the first entry.
    app.move_selection(1);
    assert_eq!(app.nav.selected_entry_index, Some(0));
    assert!(!app.show_journal_insights_preview());
}

#[test]
fn focusing_journals_shows_insights_even_with_a_lingering_entry_selection() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.select_entry_index(0);
    app.nav.focus = Focus::Entries;
    // Focused on the entry, its preview shows and its row is highlighted.
    assert!(!app.show_journal_insights_preview());
    assert!(app.entries_highlighted());

    // Moving focus back to the journal column (e.g. clicking the already-selected
    // journal, or Left from the entry) leaves the selection index untouched, but the
    // right column must revert to insights and the row must lose its highlight — the two
    // never disagree.
    app.nav.focus = Focus::Journals;
    assert_eq!(app.nav.selected_entry_index, Some(0));
    assert!(app.show_journal_insights_preview());
    assert!(!app.entries_highlighted());
}

#[test]
fn focusing_insights_shows_insights_even_with_a_lingering_entry_selection() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.select_entry_index(0);
    app.nav.focus = Focus::Entries;
    assert!(!app.show_journal_insights_preview());
    assert!(app.entries_highlighted());

    // Clicking the insights column focuses it but leaves the selection index
    // lingering. The insights panel shares the right-hand pane with the entry
    // viewer, so it must show insights and drop the row highlight — not reopen
    // the entry that was just closed.
    app.nav.focus = Focus::Insights;
    assert_eq!(app.nav.selected_entry_index, Some(0));
    assert!(app.show_journal_insights_preview());
    assert!(!app.entries_highlighted());
}

#[test]
fn hidden_journals_launch_focuses_entries_with_insights_preview() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut state = crate::config::State::default();
    state.ui.show_journals = false;
    let app = new_app_with_state(config, state);

    assert_eq!(app.nav.focus, Focus::Entries);
    assert_eq!(app.nav.selected_entry_index, None);
    assert!(app.show_journal_insights_preview());
}

#[test]
fn selected_entry_view_title_uses_entry_timestamp() {
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

    let (title, content) = app.selected_entry_view().unwrap();

    assert_eq!(title, "Wednesday, 1 July 2026, 10:23");
    assert_eq!(content, "# A\nBody\n");
}

#[test]
fn search_entry_view_title_uses_entry_timestamp() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(
        entry_dir.join("a.md"),
        "+++\n[datetime]\ncreated_at = \"2026-07-01T10:23:00+02:00\"\n+++\n\n# A\nneedle\n",
    )
    .unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");
    app.begin_search();
    app.search.query = "needle".into();
    app.update_search_results();

    let (title, content) = app.selected_entry_view().unwrap();

    assert_eq!(title, "Wednesday, 1 July 2026, 10:23");
    assert_eq!(content, "# A\nneedle\n");
}

#[test]
fn journal_focus_does_not_make_entry_targets_actionable() {
    let dir = tempdir().unwrap();
    let entry_dir = dir.path().join("work").join("2026-07-01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(entry_dir.join("a.md"), "+++\ntags = []\n+++\n\n# A\n").unwrap();

    let config = Config::new(dir.path().to_path_buf());
    let mut app = new_app(config);
    app.select_journal_by_name("work");

    app.nav.focus = Focus::Journals;
    assert!(!app.can_act_on_selected_entry());

    app.nav.focus = Focus::Entries;
    assert!(app.can_act_on_selected_entry());
}

#[test]
fn compact_width_uses_single_panel_without_inline_entry_view() {
    assert!(single_panel_is_active(TWO_PANEL_MIN_WIDTH - 1));
    assert!(!inline_entry_view_is_visible(TWO_PANEL_MIN_WIDTH - 1));
    assert!(!entry_view_is_available(TWO_PANEL_MIN_WIDTH - 1));
    assert!(entry_view_is_available(TWO_PANEL_MIN_WIDTH));
}

#[test]
fn inline_entry_view_uses_minimum_three_column_width() {
    assert!(!inline_entry_view_is_visible(
        INLINE_ENTRY_VIEW_MIN_WIDTH - 1
    ));
    assert!(inline_entry_view_is_visible(INLINE_ENTRY_VIEW_MIN_WIDTH));
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
        "+++\nfeelings = [\"calm\"]\n+++\n\n# A\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\nfeelings = [\"anxious\"]\n+++\n\n# B\n",
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
        "+++\nstarred = true\n+++\n\n# Fav\n",
    )
    .unwrap();
    fs::write(entry_dir.join("b.md"), "+++\n+++\n\n# Plain\n").unwrap();

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
        "+++\nfeelings = [\"calm\", \"excited\"]\n+++\n\n# A\n",
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
        "+++\n[location]\nname = \"Home\"\nlatitude = 52.5\nlongitude = 13.4\n+++\n\n# A\n",
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
        "+++\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nBody\n",
    )
    .unwrap();
    fs::write(
        entry_dir.join("b.md"),
        "+++\n[datetime]\ncreated_at = \"2026-07-01T11:00:00+02:00\"\n+++\n\n# B\nBody\n",
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
        "+++\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\nneedle\n",
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
        format!("+++\n[datetime]\ncreated_at = \"{created}\"\n+++\n\n{body}\n"),
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

    let first = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("x")], vec![]));
    // Same entry + width → cached rows returned, the builder isn't re-run.
    let same = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("y")], vec![]));
    assert!(Rc::ptr_eq(&first, &same));
    // A different width rebuilds.
    let narrower = app.cached_entry_body(path.as_deref(), 20, || (vec![Line::from("z")], vec![]));
    assert!(!Rc::ptr_eq(&first, &narrower));
    // Reloading the store bumps entries_version, invalidating the cache.
    app.refresh().unwrap();
    let after = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("w")], vec![]));
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
    let body = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("x")], vec![]));
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
    let body_after = app.cached_entry_body(path.as_deref(), 40, || (vec![Line::from("y")], vec![]));
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
        "+++\ntags = [\"berlin\", \"shared\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# A\n",
    )
    .unwrap();
    fs::write(
        archived_dir.join("b.md"),
        "+++\ntags = [\"wanderlust\", \"shared\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# B\n",
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
