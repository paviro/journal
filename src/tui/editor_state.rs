use std::path::PathBuf;
use std::time::Instant;

use notema_domain::Metadata;
use notema_storage::EntryRevision;
use ratatui::layout::Rect;
use ratatui_textarea::{DataCursor, TextArea, WrapMode};

use super::theme::Theme;

/// What the internal editor is writing to: an existing entry file, or a new
/// entry to be created in a journal on save.
#[derive(Clone)]
pub(crate) enum EditorTarget {
    Existing {
        journal: String,
        path: PathBuf,
        title: String,
        revision: EntryRevision,
    },
    New {
        journal: String,
    },
}

/// A modal prompt layered over the editor. Exactly one is active at a time, so
/// these are one enum rather than a set of independent booleans.
pub(crate) enum EditorPrompt {
    /// Typing normally.
    None,
    /// Answering the "Discard changes?" confirmation dialog. `discard_selected`
    /// is the highlighted button: `true` for Discard, `false` for Keep.
    ConfirmDiscard { discard_selected: bool },
    /// The "Add metadata" chooser: the next letter picks a dialog (t/p/a/f/m).
    MetadataMenu,
    /// The shortcut-reference overlay, scrolled to `scroll`.
    Help { scroll: u16 },
}

/// In-memory editing session shown inside the entry-view pane. Holds the
/// `ratatui-textarea` buffer plus enough environment to save (or discard) without
/// ever writing the body to a plaintext temp file.
pub(crate) struct EntryEditor {
    pub(crate) textarea: TextArea<'static>,
    pub(crate) target: EditorTarget,
    /// When the session opened, so elapsed writing time can be folded into the
    /// final save.
    pub(crate) start: Instant,
    /// The body as loaded, to detect unsaved changes on cancel.
    pub(crate) original: String,
    /// Buffered metadata (tags/people/activities/feelings/mood, plus carried-over
    /// starred), edited via the `^`-prefixed dialogs and written to the entry on
    /// save. Seeded from the entry for an existing edit, empty for a new one.
    pub(crate) metadata: Metadata,
    /// Metadata as loaded, so save only rewrites fields the user actually changed.
    pub(crate) original_metadata: Metadata,
    /// The modal prompt currently layered over the editor, if any.
    pub(crate) prompt: EditorPrompt,
    /// The on-screen rect of the textarea from the last render, so mouse clicks
    /// can be mapped back to a cursor position.
    pub(crate) text_rect: Rect,
    /// Whether a left-button drag is currently extending a selection.
    pub(crate) mouse_selecting: bool,
    /// The in-flight environment fetch's id (allocated from the app-level counter),
    /// or `None` when nothing is outstanding.
    pub(crate) pending_environment: Option<u64>,
    /// The landed environment for the current location, attached to the entry on save.
    pub(crate) environment: Option<crate::tui::environment::Environment>,
    /// The timezone of the current location, when `use_location_timezone` resolved
    /// one (new entries only). Re-zones the environment datetime and, on save,
    /// stamps `created_at`/`edited_at`/`timezone` to the place rather than the system.
    pub(crate) zone: Option<chrono_tz::Tz>,
    /// The body text the syntax highlighting was last computed for, so the map is
    /// only recomputed when the body actually changes rather than every frame.
    last_highlight_src: Option<String>,
}

impl EntryEditor {
    pub(crate) fn for_existing(
        journal: String,
        path: PathBuf,
        title: String,
        revision: EntryRevision,
        body: &str,
        metadata: Metadata,
    ) -> Self {
        Self {
            textarea: new_textarea(body, None),
            target: EditorTarget::Existing {
                journal,
                path,
                title,
                revision,
            },
            start: Instant::now(),
            original: body.to_string(),
            original_metadata: metadata.clone(),
            metadata,
            prompt: EditorPrompt::None,
            text_rect: Rect::default(),
            mouse_selecting: false,
            pending_environment: None,
            environment: None,
            zone: None,
            last_highlight_src: None,
        }
    }

    pub(crate) fn for_new(journal: String) -> Self {
        Self {
            textarea: new_textarea("", Some("Write your entry…")),
            target: EditorTarget::New { journal },
            start: Instant::now(),
            original: String::new(),
            metadata: Metadata::default(),
            original_metadata: Metadata::default(),
            prompt: EditorPrompt::None,
            text_rect: Rect::default(),
            mouse_selecting: false,
            pending_environment: None,
            environment: None,
            zone: None,
            last_highlight_src: None,
        }
    }

    /// Recompute markdown syntax highlighting and hand it to the textarea, but only
    /// when the body changed since the last call. Cheap to call every frame.
    pub(crate) fn refresh_syntax_highlight(&mut self, theme: &Theme) {
        let body = self.text();
        if self.last_highlight_src.as_deref() != Some(body.as_str()) {
            let spans = super::editor_highlight::highlight_body(theme, &body);
            self.textarea.set_syntax_spans(spans);
            self.last_highlight_src = Some(body);
        }
    }

    /// Map an absolute screen position to a data `(row, col)` cursor position
    /// within the textarea, or `None` when the point is outside the text rect.
    /// Accounts for soft-wrap and the current scroll offset, so it stays exact
    /// once the body scrolls.
    pub(crate) fn text_pos_at(&self, col: u16, row: u16) -> Option<(u16, u16)> {
        let rect = self.text_rect;
        if rect.width == 0
            || col < rect.x
            || row < rect.y
            || col >= rect.x + rect.width
            || row >= rect.y + rect.height
        {
            return None;
        }
        let screen_row = self.textarea.scroll_offset() as usize + (row - rect.y) as usize;
        let screen_col = (col - rect.x) as usize;
        let DataCursor(line, column) = self.textarea.cursor_at_screen(screen_row, screen_col);
        Some((line as u16, column as u16))
    }

    /// Scroll the body by `delta` wrapped rows (negative is up), clamped so the
    /// last line can't scroll above the bottom edge into empty space. The crate's
    /// own delta scroll saturating-adds with no content bound, so we clamp here.
    pub(crate) fn scroll_lines(&mut self, delta: i16) {
        let height = self.text_rect.height as usize;
        let max_top = self.textarea.screen_line_count().saturating_sub(height) as i64;
        let cur = self.textarea.scroll_offset() as i64;
        let target = (cur + delta as i64).clamp(0, max_top);
        if target != cur {
            self.textarea.scroll(((target - cur) as i16, 0));
        }
    }

    /// The current buffer joined back into a single body string.
    pub(crate) fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Whether the buffer differs from what was loaded — body text or any of the
    /// buffered metadata, since metadata edits are part of the editor's changes.
    pub(crate) fn is_dirty(&self) -> bool {
        self.text() != self.original || self.metadata != self.original_metadata
    }

    /// The pane title, signalling edit vs compose mode.
    pub(crate) fn title(&self) -> &str {
        match &self.target {
            EditorTarget::Existing { .. } => "Edit entry",
            EditorTarget::New { .. } => "New entry",
        }
    }
}

fn new_textarea(body: &str, placeholder: Option<&str>) -> TextArea<'static> {
    let theme = Theme::terminal_default();
    let mut textarea = TextArea::new(body.split('\n').map(str::to_string).collect());
    // The cursor line defaults to no highlight, keeping the journal body
    // reading like the viewer rather than a code editor; themes may tint it.
    textarea.set_cursor_line_style(theme.cursor_line());
    // Make selections visible (reversed video) so keyboard/mouse selection reads
    // clearly and can't silently swallow text.
    textarea.set_selection_style(theme.selection());
    // Soft-wrap long lines like the viewer, splitting mid-word only when a word is
    // wider than the pane.
    textarea.set_wrap_mode(WrapMode::WordOrGlyph);
    if let Some(text) = placeholder {
        textarea.set_placeholder_text(text.to_string());
        // The widget's own default is a hardcoded dark gray; the theme's
        // placeholder ink matches the single-line fields instead.
        textarea.set_placeholder_style(theme.placeholder());
    }
    textarea
}
