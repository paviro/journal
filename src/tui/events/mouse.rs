use crate::AppResult;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, entry_view_is_available, inline_entry_view_is_visible},
    events::actions::view_selected,
    render,
};

use super::action::Action;

pub(crate) fn handle_mouse(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
) -> AppResult<bool> {
    let size = terminal.size()?;
    let area = Rect::new(0, 0, size.width, size.height);

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        if app.has_overlay() {
            handle_dialog_hint_click(terminal, app, mouse, area)?;
            return Ok(false);
        }
        let layout = render::tui_layout(area, app);
        if render::point_in_rect(layout.footer, mouse.column, mouse.row) {
            if let Some(action) = footer_click_to_action(app, mouse, layout) {
                return super::dispatch_action(terminal, app, action);
            }
            return Ok(false);
        }
    }

    handle_mouse_in_area(app, mouse, area)?;
    Ok(false)
}

pub(super) fn handle_mouse_in_area(app: &mut App, mouse: MouseEvent, area: Rect) -> AppResult<()> {
    if app.has_overlay() {
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
        let text_width = area.width.saturating_sub(11);
        let rows = render::entry_row_metadata(app, text_width);
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
        let tags = app.selected_entry_tags();
        let feelings = app.selected_entry_feelings();
        if let Some(feeling) =
            render::feeling_at_point(area, mouse.column, mouse.row, &tags, &feelings)
        {
            app.begin_feeling_search(&feeling);
            return Ok(());
        }
        if let Some(tag) = render::tag_at_point(area, mouse.column, mouse.row, &tags, &feelings) {
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
        let text_width = area.width.saturating_sub(11);
        let rows = render::entry_row_metadata(app, text_width);
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

// ── Footer click ──────────────────────────────────────────────────────────────

fn footer_click_to_action(
    app: &App,
    mouse: MouseEvent,
    layout: render::TuiLayout,
) -> Option<Action> {
    let text = if app.entry_view_expanded {
        " close (enter/esc) | edit (e) | quit (q)".to_string()
    } else {
        render::footer_text(app, layout.entry_view_visible)
    };

    let seg = hint_segment_at(&text, layout.footer.x, mouse.column)?;

    if seg.starts_with("new journal") {
        Some(Action::NewJournal)
    } else if seg.starts_with("new entry") {
        Some(Action::NewEntry)
    } else if seg == "refresh (r)" {
        Some(Action::Refresh)
    } else if seg.starts_with("edit tags") && app.has_selected_entry_target() {
        Some(Action::BeginEditTags)
    } else if seg.starts_with("edit feelings") && app.has_selected_entry_target() {
        Some(Action::BeginEditFeelings)
    } else if seg.starts_with("edit mood") && app.has_selected_entry_target() {
        Some(Action::BeginEditMood)
    } else if seg.starts_with("edit") && app.can_act_on_selected_entry() {
        Some(Action::EditSelected)
    } else if seg.starts_with("view") && app.has_selected_entry_target() {
        Some(Action::ViewSelected)
    } else if seg.starts_with("delete") && app.has_selected_entry_target() {
        Some(Action::BeginDelete)
    } else if seg.starts_with("close") && app.entry_view_expanded {
        Some(Action::CancelOverlay)
    } else if seg.starts_with("quit") {
        Some(Action::Quit)
    } else if seg.starts_with("exit search") {
        Some(Action::ExitSearch)
    } else if seg.starts_with("search") {
        Some(Action::BeginSearch)
    } else {
        None
    }
}

// ── Dialog hint click routing ─────────────────────────────────────────────────

/// Returns the trimmed hint segment under `col`, given that the hint string
/// starts at `origin_x` on screen. Separators are `" | "` (3 columns).
fn hint_segment_at(hint: &str, origin_x: u16, col: u16) -> Option<&str> {
    if col < origin_x {
        return None;
    }
    let rel = (col - origin_x) as usize;
    let mut x = 0usize;
    for seg in hint.split(" | ") {
        let width = seg.chars().count();
        if rel >= x && rel < x + width {
            return Some(seg.trim());
        }
        x += width + 3;
    }
    None
}

fn handle_dialog_hint_click(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
    area: Rect,
) -> AppResult<()> {
    let col = mouse.column;
    let row = mouse.row;

    if let Some(focus) = app.edit_tag_state().map(|s| s.focus) {
        let filtered_len = app.edit_tag_state().map_or(0, |s| s.filtered.len());
        let dialog = render::tags_dialog_area(area, filtered_len);
        let inner = render::panel_inner(dialog);
        if row == inner.y + inner.height.saturating_sub(1)
            && let Some(seg) = hint_segment_at(render::tags_dialog_hint(focus), inner.x, col)
            && let Some(action) = tags_hint_to_action(app, seg)
        {
            super::dispatch_action(terminal, app, action)?;
        }
        return Ok(());
    }

    if app.edit_feeling_state().is_some() {
        let all_len = app.edit_feeling_state().map_or(0, |s| s.all_feelings.len());
        let dialog = render::feelings_dialog_area(area, all_len);
        let inner = render::panel_inner(dialog);
        if row == inner.y + inner.height.saturating_sub(1)
            && let Some(seg) = hint_segment_at(render::FEELINGS_HINT, inner.x, col)
            && let Some(action) = feelings_hint_to_action(seg)
        {
            super::dispatch_action(terminal, app, action)?;
        }
        return Ok(());
    }

    if app.edit_mood_state().is_some() {
        let dialog = render::mood_dialog_area(area);
        let inner = render::panel_inner(dialog);
        if row == inner.y + inner.height.saturating_sub(1)
            && let Some(seg) = hint_segment_at(render::MOOD_HINT, inner.x, col)
            && let Some(action) = mood_hint_to_action(seg)
        {
            super::dispatch_action(terminal, app, action)?;
        }
    }

    Ok(())
}

/// Pure: maps a tags-dialog hint segment to an Action.
fn tags_hint_to_action(app: &App, seg: &str) -> Option<Action> {
    if seg.starts_with("toggle") && app.edit_tag_state().is_some_and(|s| !s.filtered.is_empty()) {
        Some(Action::TagsToggle)
    } else if seg.starts_with("input") || seg.starts_with("list") {
        Some(Action::TagsSwitchFocus)
    } else if seg.starts_with("add") {
        Some(Action::TagsAddFromInput)
    } else if seg.starts_with("save") {
        Some(Action::TagsSave)
    } else {
        Some(Action::CancelOverlay)
    }
}

/// Pure: maps a feelings-dialog hint segment to an Action.
fn feelings_hint_to_action(seg: &str) -> Option<Action> {
    if seg.starts_with("toggle") {
        Some(Action::FeelingsToggle)
    } else if seg.starts_with("save") {
        Some(Action::FeelingsSave)
    } else {
        Some(Action::CancelOverlay)
    }
}

/// Pure: maps a mood-dialog hint segment to an Action.
fn mood_hint_to_action(seg: &str) -> Option<Action> {
    if seg.starts_with("decrease") {
        Some(Action::MoodDecrease)
    } else if seg.starts_with("increase") {
        Some(Action::MoodIncrease)
    } else if seg.starts_with("save") {
        Some(Action::MoodSave)
    } else if seg.starts_with("clear") {
        Some(Action::MoodClear)
    } else {
        Some(Action::CancelOverlay)
    }
}

