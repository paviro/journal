use crate::AppResult;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, entry_view_is_available},
    events::actions::view_selected,
    render,
};

pub(crate) fn handle_mouse(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
) -> AppResult<()> {
    let size = terminal.size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    handle_mouse_in_area(app, mouse, area)
}

pub(super) fn handle_mouse_in_area(app: &mut App, mouse: MouseEvent, area: Rect) -> AppResult<()> {
    if app.new_journal_input.is_some() || app.confirm_delete {
        return Ok(());
    }

    app.normalize_focus(entry_view_is_available(area.width));
    let layout = render::tui_layout(area, app);

    if app.viewer.is_some() {
        match mouse.kind {
            MouseEventKind::ScrollUp => scroll_viewer(app, -1),
            MouseEventKind::ScrollDown => scroll_viewer(app, 1),
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
            app.journal_scroll,
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
            render::entry_index_at(area, mouse.column, mouse.row, app.entry_scroll, &rows)
        {
            app.select_entry_index(index);
            if !layout.entry_view_visible {
                view_selected(app)?;
            }
        }
        return Ok(());
    }

    if let Some(area) = layout.entry_view
        && render::point_in_rect(area, mouse.column, mouse.row)
        && app.has_selected_entry_target()
    {
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
        app.entry_scroll = render::scroll_offset(
            app.entry_scroll,
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
        app.journal_scroll = render::scroll_offset(
            app.journal_scroll,
            delta,
            app.journals.len(),
            render::panel_inner(area).height,
        );
    }
}

fn scroll_viewer(app: &mut App, delta: i16) {
    let Some(viewer) = app.viewer.as_mut() else {
        return;
    };

    if delta.is_negative() {
        viewer.scroll = viewer.scroll.saturating_sub(delta.unsigned_abs());
    } else {
        viewer.scroll = viewer.scroll.saturating_add(delta as u16);
    }
}
