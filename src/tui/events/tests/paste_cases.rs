//! Bracketed-paste routing: a pasted block lands in the caret's text sink in one
//! edit, and single-line fields fold newlines instead of splitting.

use super::*;

/// A `TestBackend` terminal to drive `handle_paste`, which only needs it for the
/// generic `dispatch_action` path.
fn test_terminal() -> ratatui::Terminal<ratatui::backend::TestBackend> {
    ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap()
}

#[test]
fn paste_into_editor_inserts_a_multiline_block() {
    let mut app = app_with_journals(&["work"]);
    app.open_editor_for_new();
    let mut terminal = test_terminal();

    handle_paste(&mut terminal, &mut app, "line one\nline two".to_string()).unwrap();

    // A real block insert keeps the newline (the editor is multi-line); replaying
    // it as key events never would, since Enter isn't part of pasted text.
    assert_eq!(app.editor.as_ref().unwrap().text(), "line one\nline two");
}

#[test]
fn paste_into_search_field_folds_newlines_onto_one_line() {
    let mut app = app_with_entries(1);
    app.begin_search();
    let mut terminal = test_terminal();

    handle_paste(&mut terminal, &mut app, "hello\nworld".to_string()).unwrap();

    assert_eq!(app.search.query.as_str(), "hello world");
}

#[test]
fn paste_with_no_focused_field_is_inert() {
    let mut app = app_with_entries(1);
    // Browse mode, no editor, no overlay: nothing owns the caret.
    let mut terminal = test_terminal();

    handle_paste(&mut terminal, &mut app, "ignored".to_string()).unwrap();

    assert!(app.editor.is_none());
    assert!(app.search.query.is_empty());
}
