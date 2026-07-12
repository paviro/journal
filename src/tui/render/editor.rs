use ratatui::{Frame, layout::Rect};

use crate::tui::{
    editor_state::{EditorPrompt, EntryEditor},
    render::{entry_metadata_layout, panel_block, render_scrollbar_if_needed},
    surface::PanelGeometry,
    theme::theme,
};

use super::metadata::{EntryMetadata, draw_metadata_section};
use super::reader::metadata_scrolls_with_body;

/// Draw the internal editor in the entry-view pane: the same bordered panel as
/// the viewer, with the `ratatui-textarea` buffer as the body and the buffered
/// metadata pinned below it. Honors the viewer's max-width and vertical-center
/// settings, and shows an inline discard confirmation when one is pending.
pub(crate) fn draw_entry_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &mut EntryEditor,
    side_margin: u16,
    top_margin: u16,
) {
    let block = panel_block(editor.title(), true, None);
    frame.render_widget(block, area);
    super::panel_focus_stripe(frame, area, true);

    // Same builder the viewer uses, from the buffered metadata — so location and
    // every other front-matter field show in edit mode too.
    let metadata = EntryMetadata::from_metadata(&editor.metadata);

    // The metadata section pins below the body only while the pane can still give the
    // body its minimum height; once the metadata would push it under that, it's
    // dropped and the whole pane goes to the textarea. (The viewer instead folds
    // metadata into its scroll there, but the editor's scroll is cursor-driven and
    // can't reach a read-only block past the text.) Nothing is lost: the Ctrl+G
    // dialogs show the current values as you edit them, and the viewer shows them in
    // full on save.
    let (body_area, layout) = if metadata_scrolls_with_body(area, metadata.values()) {
        (PanelGeometry::new(area).content, None)
    } else {
        let layout = entry_metadata_layout(area, metadata.values());
        (layout.content, Some(layout))
    };

    // Inset the writing area with a fixed margin (side left/right, top, 0 bottom)
    // rather than a max-width gutter. The editor never floats vertically — typing
    // at a moving baseline is disorienting.
    let text_rect = Rect {
        x: body_area.x + side_margin,
        y: body_area.y + top_margin,
        width: body_area.width.saturating_sub(side_margin * 2),
        height: body_area.height.saturating_sub(top_margin),
    };

    // While selecting, draw the reversed-block caret so the boundary character
    // reads as part of the selection (a thin bar can't fill that cell); otherwise
    // the theme's cursor style — by default unstyled, leaving the native bar
    // cursor placed below as the only caret.
    let selecting = editor.textarea.selection_range().is_some();
    editor.textarea.set_cursor_style(if selecting {
        theme().selection()
    } else {
        theme().cursor()
    });

    editor.text_rect = text_rect;
    frame.render_widget(&editor.textarea, text_rect);

    // Native terminal bar cursor, only while typing without a selection and with
    // no modal prompt over the editor. screen_cursor().row is the absolute wrapped
    // row; subtracting the scroll top gives the viewport-relative row. Wrap mode
    // has no horizontal scroll, so col maps directly. Valid only after render.
    if !selecting && matches!(editor.prompt, EditorPrompt::None) {
        let sc = editor.textarea.screen_cursor();
        let scroll = editor.textarea.scroll_offset() as usize;
        if let Some(rel) = sc.row.checked_sub(scroll) {
            let x = text_rect.x + sc.col as u16;
            let y = text_rect.y + rel as u16;
            if x < text_rect.x + text_rect.width && y < text_rect.y + text_rect.height {
                frame.set_cursor_position((x, y));
            }
        }
    }

    // Scroll offset and wrapped-line count are only valid after the textarea has
    // rendered (it stores them during render), so read them here.
    render_scrollbar_if_needed(
        frame,
        area,
        editor.textarea.screen_line_count(),
        text_rect.height,
        editor.textarea.scroll_offset() as usize,
        // The editor is always the active surface while shown.
        true,
    );

    if let Some(layout) = layout {
        draw_metadata_section(frame, layout, &metadata);
    }
}
