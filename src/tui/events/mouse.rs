use crate::AppResult;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, entry_view_is_available, inline_entry_view_is_visible},
    events::actions::{create_entry_in_selected_journal, edit_selected, view_selected},
    render,
};

pub(crate) fn handle_mouse(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
) -> AppResult<bool> {
    let size = terminal.size()?;
    let area = Rect::new(0, 0, size.width, size.height);

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        let layout = render::tui_layout(area, app);
        if render::point_in_rect(layout.footer, mouse.column, mouse.row) {
            return handle_footer_click(terminal, app, mouse, layout);
        }
    }

    handle_mouse_in_area(app, mouse, area)?;
    Ok(false)
}

pub(super) fn handle_mouse_in_area(app: &mut App, mouse: MouseEvent, area: Rect) -> AppResult<()> {
    if app.new_journal_input().is_some()
        || app.is_confirming_delete()
        || app.edit_tag_state().is_some()
    {
        return Ok(());
    }

    app.normalize_focus(entry_view_is_available(area.width));
    let layout = render::tui_layout(area, app);

    if app.entry_view_expanded {
        match mouse.kind {
            MouseEventKind::ScrollUp => app.scroll_entry_view(-1),
            MouseEventKind::ScrollDown => app.scroll_entry_view(1),
            _ => {}
        }
        return Ok(());
    }

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => handle_left_click(app, mouse, layout)?,
        MouseEventKind::ScrollUp => handle_wheel(app, mouse, layout, -1),
        MouseEventKind::ScrollDown => handle_wheel(app, mouse, layout, 1),
        _ => {}
    }

    Ok(())
}

fn handle_left_click(app: &mut App, mouse: MouseEvent, layout: render::TuiLayout) -> AppResult<()> {
    if app.mode == Mode::Browse
        && let Some(area) = layout.journals
        && render::point_in_rect(area, mouse.column, mouse.row)
    {
        app.focus = if layout.single_panel {
            Focus::Entries
        } else {
            Focus::Journals
        };
        if let Some(index) = render::journal_index_at(
            area,
            mouse.column,
            mouse.row,
            app.scroll.journal,
            app.journals.len(),
        ) {
            app.select_journal(index);
        }
        return Ok(());
    }

    if let Some(area) = layout.entries
        && render::point_in_rect(area, mouse.column, mouse.row)
    {
        app.focus = Focus::Entries;
        let rows = render::entry_row_metadata(app);
        if let Some(index) =
            render::entry_index_at(area, mouse.column, mouse.row, app.scroll.entry, &rows)
        {
            app.select_entry_index(index);
            if !inline_entry_view_is_visible(layout.content.width) {
                view_selected(app)?;
            }
        }
        return Ok(());
    }

    if let Some(area) = layout.entry_view
        && render::point_in_rect(area, mouse.column, mouse.row)
        && app.has_selected_entry_target()
    {
        if let Some(tag) = {
            let tags = app.selected_entry_tags();
            render::tag_at_point(area, mouse.column, mouse.row, &tags)
        } {
            app.begin_tag_search(&tag);
            return Ok(());
        }
        app.focus = Focus::EntryView;
    }

    Ok(())
}

fn handle_wheel(app: &mut App, mouse: MouseEvent, layout: render::TuiLayout, delta: i16) {
    if let Some(area) = layout.entry_view
        && render::point_in_rect(area, mouse.column, mouse.row)
    {
        app.focus = Focus::EntryView;
        app.scroll_entry_view(delta);
        return;
    }

    if let Some(area) = layout.entries
        && render::point_in_rect(area, mouse.column, mouse.row)
    {
        let rows = render::entry_row_metadata(app);
        app.scroll.entry = render::scroll_offset(
            app.scroll.entry,
            delta,
            render::total_entry_row_height(&rows),
            render::panel_inner(area).height,
        );
        return;
    }

    if app.mode == Mode::Browse
        && let Some(area) = layout.journals
        && render::point_in_rect(area, mouse.column, mouse.row)
    {
        app.scroll.journal = render::scroll_offset(
            app.scroll.journal,
            delta,
            app.journals.len(),
            render::panel_inner(area).height,
        );
    }
}

fn handle_footer_click(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
    layout: render::TuiLayout,
) -> AppResult<bool> {
    let text = if app.entry_view_expanded {
        " close (enter/esc) | edit (e) | quit (q)".to_string()
    } else {
        render::footer_text(app, layout.entry_view_visible)
    };

    let segments: Vec<&str> = text.split(" | ").collect();
    let footer_x = layout.footer.x;
    let click_x = mouse.column;

    let mut x_pos = footer_x;
    for segment in &segments {
        let seg_len = segment.len() as u16;
        if click_x >= x_pos && click_x < x_pos + seg_len {
            let seg = segment.trim();
            if seg.starts_with("new journal") {
                app.begin_new_journal_input();
            } else if seg.starts_with("new entry") {
                create_entry_in_selected_journal(terminal, app)?;
            } else if seg == "refresh (r)" {
                app.refresh()?;
            } else if seg.starts_with("edit") && app.can_act_on_selected_entry() {
                edit_selected(terminal, app)?;
            } else if seg.starts_with("view") && app.has_selected_entry_target() {
                view_selected(app)?;
            } else if seg.starts_with("delete") && app.has_selected_entry_target() {
                app.begin_confirm_delete();
            } else if seg.starts_with("edit tags") && app.has_selected_entry_target() {
                app.begin_edit_tags();
            } else if seg.starts_with("close") && app.entry_view_expanded {
                app.entry_view_expanded = false;
                app.focus = Focus::Entries;
            } else if seg.starts_with("quit") {
                return Ok(true);
            } else if seg.starts_with("exit search") {
                app.exit_search();
            } else if seg.starts_with("search") {
                app.begin_search();
            }
            return Ok(false);
        }
        x_pos += seg_len + 3;
    }

    Ok(false)
}
