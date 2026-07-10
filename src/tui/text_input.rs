//! Single-line text inputs.
//!
//! [`TextInput`] wraps the editor's [`TextArea`] so every plain text field
//! (search, dialog filters, location query/name, the new-journal name) shares
//! the editor's editing model: caret movement, shift+arrow and mouse selection,
//! ctrl word ops, and the native bar cursor.
//!
//! [`PassphraseInput`] stays a minimal hand-rolled buffer: the textarea keeps
//! undo and yank history, which would retain a typed passphrase after the
//! unlock screen zeroizes it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui_textarea::{CursorMove, TextArea};
use std::ops::{Deref, DerefMut};
use zeroize::Zeroize;

pub(crate) struct TextInput {
    textarea: TextArea<'static>,
    /// Where the field was last drawn, for mouse hit-testing. `Rect::default()`
    /// (zero-sized) until the first render, which no click can hit.
    last_area: Rect,
}

impl TextInput {
    pub(crate) fn as_str(&self) -> &str {
        &self.textarea.lines()[0]
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    /// Feed a key press. Enter and Tab are commands (submit / focus switch),
    /// never text — the field stays a single line — and Ctrl+A selects all
    /// (shadowing the textarea's emacs-style line-start, which Home covers).
    /// Everything else gets the textarea's editing model. Returns whether the
    /// text changed, so callers can run their after-edit hooks only on edits.
    pub(crate) fn input(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('a') => {
                    self.textarea.select_all();
                    return false;
                }
                // The fork maps Ctrl+M to insert_newline (terminals that
                // disambiguate report it as a plain char + CONTROL).
                KeyCode::Char('m') => return false,
                _ => {}
            }
        }
        match key.code {
            // Raw newline chars are rejected too, in case a terminal protocol
            // ever reports Enter as a plain char.
            KeyCode::Enter | KeyCode::Tab | KeyCode::Char('\n' | '\r') => false,
            _ => self.textarea.input(key),
        }
    }

    /// Empty the field, keeping its configuration (placeholder, styles).
    pub(crate) fn clear(&mut self) {
        self.set_text("");
    }

    /// Replace the text, keeping the field's placeholder; the caret lands at
    /// the end. Rebuilds the textarea so programmatic transitions (seeding or
    /// clearing a query) don't pile into the undo/yank history, where Ctrl+U
    /// or Ctrl+Y could resurrect them mid-session.
    pub(crate) fn set_text(&mut self, text: &str) {
        let mut fresh = Self::from(text);
        fresh.set_placeholder_text(self.textarea.placeholder_text().to_string());
        fresh.last_area = self.last_area;
        *self = fresh;
    }

    /// Whether the caret sits at the end of the text.
    pub(crate) fn cursor_at_end(&self) -> bool {
        self.textarea.cursor().1 >= self.as_str().chars().count()
    }

    /// Viewport-relative caret column after the last render, for placing the
    /// native cursor. The fork doesn't expose the horizontal scroll offset, so
    /// this clamps to the last cell — exact while the text fits the field or
    /// the caret rides the end (the typing case).
    // ponytail: off by the scroll offset for mid-string edits in overflowing
    // text; expose the column offset in the ratatui-textarea fork if it matters.
    pub(crate) fn visible_cursor_col(&self, width: u16) -> u16 {
        (self.textarea.screen_cursor().col as u16).min(width.saturating_sub(1))
    }

    /// Jump the caret to a click `col` cells into the field. Same horizontal
    /// scroll caveat as [`Self::visible_cursor_col`].
    fn jump_to_col(&mut self, col: u16) {
        let cursor = self.textarea.cursor_at_screen(0, col as usize);
        self.textarea
            .move_cursor(CursorMove::Jump(cursor.0 as u16, cursor.1 as u16));
    }

    /// Mouse down in the field: place the caret and arm a selection, mirroring
    /// the editor's click flow.
    pub(crate) fn begin_mouse_selection(&mut self, col: u16) {
        self.textarea.cancel_selection();
        self.jump_to_col(col);
        self.textarea.start_selection();
    }

    /// Mouse drag: extend the armed selection to the dragged column.
    pub(crate) fn drag_mouse_selection(&mut self, col: u16) {
        self.jump_to_col(col);
    }

    /// Mouse up: a click without a drag leaves an empty selection — cancel it
    /// so the reversed selection style doesn't linger on the caret cell.
    pub(crate) fn end_mouse_selection(&mut self) {
        if self
            .textarea
            .selection_range()
            .is_none_or(|(start, end)| start == end)
        {
            self.textarea.cancel_selection();
        }
    }

    /// Draw the field into `rect` and, when `focused`, place the native bar
    /// cursor at the caret. While a selection is active the native cursor is
    /// hidden and the widget draws a reversed-block caret instead, so the
    /// boundary character reads as part of the selection — exactly like the
    /// editor (a thin bar can't fill that cell). Remembers `rect` for mouse
    /// hit-testing.
    ///
    /// The field's style (e.g. an underline) is painted across the whole rect
    /// first: the widget styles only the glyphs it draws, so an empty tail —
    /// or a placeholder — would otherwise drop the form-field look. While the
    /// field is empty the whole line dims to match the placeholder, switching
    /// to full intensity once the user types.
    pub(crate) fn render_in(&mut self, frame: &mut Frame<'_>, rect: Rect, focused: bool) {
        self.last_area = rect;
        if rect.width == 0 || rect.height == 0 {
            return;
        }

        let selecting = self.textarea.selection_range().is_some();
        // The reversed-block caret marks the selection boundary, but only while
        // it sits on a real character: past the last char it would paint the
        // phantom end-of-line cell as a selected trailing space.
        self.textarea
            .set_cursor_style(if selecting && !self.cursor_at_end() {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            });

        let mut style = self.textarea.style();
        if self.is_empty() {
            style = style.add_modifier(Modifier::DIM);
        }
        frame.buffer_mut().set_style(rect, style);
        frame.render_widget(&self.textarea, rect);
        if focused && !selecting {
            frame.set_cursor_position((rect.x + self.visible_cursor_col(rect.width), rect.y));
        }
    }

    pub(crate) fn last_area(&self) -> Rect {
        self.last_area
    }

    /// Drop the remembered rect. Called at the top of every frame; a field
    /// that isn't re-drawn (panel hidden by a fullscreen pane) must not keep
    /// swallowing clicks at its stale coordinates.
    pub(crate) fn forget_area(&mut self) {
        self.last_area = Rect::default();
    }

    /// Column into the field for a click at screen `(col, row)`, if it hits the
    /// field as last drawn.
    pub(crate) fn hit_col(&self, col: u16, row: u16) -> Option<u16> {
        let rect = self.last_area;
        (rect.width > 0 && crate::tui::surface::point_in_rect(rect, col, row)).then(|| col - rect.x)
    }
}

impl Default for TextInput {
    fn default() -> Self {
        Self::from(String::new())
    }
}

/// Seed a field with existing text; the caret lands at the end.
impl From<String> for TextInput {
    fn from(text: String) -> Self {
        let mut textarea = TextArea::new(vec![text]);
        // Match the entry editor's look: no cursor-line highlight, reversed
        // selections, and the widget's own block cursor hidden — the native
        // terminal bar cursor marks the caret instead.
        textarea.set_cursor_line_style(Style::default());
        textarea.set_selection_style(Style::default().add_modifier(Modifier::REVERSED));
        textarea.set_cursor_style(Style::default());
        // Every field shares the form look: underlined, with a dim placeholder.
        textarea.set_style(Style::default().add_modifier(Modifier::UNDERLINED));
        textarea.set_placeholder_style(Style::default().add_modifier(Modifier::DIM));
        textarea.move_cursor(CursorMove::End);
        Self {
            textarea,
            last_area: Rect::default(),
        }
    }
}

impl From<&str> for TextInput {
    fn from(text: &str) -> Self {
        Self::from(text.to_string())
    }
}

/// The wrapped textarea's API (widget rendering, placeholder/style setters,
/// selection state) stays reachable; only editing goes through
/// [`TextInput::input`] to keep the field single-line.
impl Deref for TextInput {
    type Target = TextArea<'static>;

    fn deref(&self) -> &Self::Target {
        &self.textarea
    }
}

impl DerefMut for TextInput {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.textarea
    }
}

/// The unlock passphrase buffer: text plus a char-index caret, wiped with
/// [`Zeroize`] as soon as the passphrase has been used.
#[derive(Default)]
pub(crate) struct PassphraseInput {
    text: String,
    /// Caret position as a char index, in `0..=text.chars().count()`.
    cursor: usize,
}

impl PassphraseInput {
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    /// Caret position as a char index.
    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    /// Byte offset of the caret, clamped to the end.
    fn cursor_byte(&self) -> usize {
        self.text
            .char_indices()
            .nth(self.cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(self.text.len())
    }

    pub(crate) fn insert(&mut self, ch: char) {
        let byte = self.cursor_byte();
        self.text.insert(byte, ch);
        self.cursor += 1;
    }

    pub(crate) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let byte = self.cursor_byte();
        self.text.remove(byte);
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(crate) fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    /// Place the caret from a click `col` cells into the field (the mask is one
    /// cell per char, so the column is the char index), clamped to the end.
    pub(crate) fn click_at(&mut self, col: u16) {
        self.cursor = (col as usize).min(self.text.chars().count());
    }
}

impl Zeroize for PassphraseInput {
    fn zeroize(&mut self) {
        self.text.zeroize();
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn enter_and_tab_never_edit_the_field() {
        let mut input = TextInput::from("ab");
        assert!(!input.input(key(KeyCode::Enter)));
        assert!(!input.input(key(KeyCode::Tab)));
        assert_eq!(input.as_str(), "ab");
        assert_eq!(input.lines().len(), 1, "stays a single line");
    }

    #[test]
    fn arrows_move_the_caret_for_mid_string_edits() {
        let mut input = TextInput::from("rst");
        input.input(key(KeyCode::Left));
        input.input(key(KeyCode::Left));
        assert!(input.input(key(KeyCode::Char('u'))));
        assert_eq!(input.as_str(), "rust");
    }

    #[test]
    fn shift_arrow_selects_and_typing_replaces_the_selection() {
        let mut input = TextInput::from("abc");
        input.input(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
        assert!(input.selection_range().is_some());
        input.input(key(KeyCode::Char('d')));
        assert_eq!(input.as_str(), "abd");
    }

    #[test]
    fn clear_empties_but_keeps_the_placeholder() {
        let mut input = TextInput::from("hello");
        input.set_placeholder_text("type to search");
        input.clear();
        assert!(input.is_empty());
        assert_eq!(input.placeholder_text(), "type to search");
    }

    #[test]
    fn seeding_places_the_caret_at_the_end() {
        let mut input = TextInput::from("täg");
        assert!(input.input(key(KeyCode::Char('!'))));
        assert_eq!(input.as_str(), "täg!");
    }

    #[test]
    fn passphrase_edits_respect_char_boundaries_and_zeroize() {
        let mut input = PassphraseInput::default();
        for ch in "aöc".chars() {
            input.insert(ch);
        }
        input.move_left();
        input.insert('ü');
        assert_eq!(input.as_str(), "aöüc");
        input.backspace();
        assert_eq!(input.as_str(), "aöc");
        input.move_right();
        input.move_right(); // clamped at the end

        input.zeroize();
        assert!(input.as_str().is_empty());
        assert_eq!(input.cursor(), 0);
    }
}
