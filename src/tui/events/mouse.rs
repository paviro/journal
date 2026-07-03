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

    if app.has_overlay() {
        handle_overlay_mouse(Some(terminal), app, mouse, area)?;
        return Ok(false);
    }

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
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
        handle_overlay_mouse(None, app, mouse, area)?;
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
        && render::point_in_rect(area.area, mouse.column, mouse.row)
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
        && render::point_in_rect(area.panel.area, mouse.column, mouse.row)
    {
        app.focus = Focus::Entries;
        let rows = render::entry_row_metadata(app, area.text_width);
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
        && render::point_in_rect(area.area, mouse.column, mouse.row)
        && app.has_selected_entry_target()
    {
        let tags = app.selected_entry_tags();
        let feelings = app.selected_entry_feelings();
        let mood = app.selected_entry_mood();
        if let Some(feeling) =
            render::feeling_at_point(area.area, mouse.column, mouse.row, &tags, &feelings, mood)
        {
            app.begin_feeling_search(&feeling);
            return Ok(());
        }
        if let Some(tag) =
            render::tag_at_point(area.area, mouse.column, mouse.row, &tags, &feelings, mood)
        {
            app.begin_tag_search(&tag);
            return Ok(());
        }
        app.focus = Focus::EntryView;
    }

    Ok(())
}

fn handle_wheel(app: &mut App, mouse: MouseEvent, layout: render::TuiLayout, delta: i16) {
    if let Some(area) = layout.entry_view
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        app.focus = Focus::EntryView;
        app.scroll_entry_view(delta);
        return;
    }

    if let Some(area) = layout.entries
        && render::point_in_rect(area.panel.area, mouse.column, mouse.row)
    {
        let rows = render::entry_row_metadata(app, area.text_width);
        app.scroll.entry = render::scroll_offset(
            app.scroll.entry,
            delta,
            render::total_entry_row_height(&rows),
            area.viewport_height,
        );
        return;
    }

    if app.mode == Mode::Browse
        && let Some(area) = layout.journals
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        app.scroll.journal = render::scroll_offset(
            app.scroll.journal,
            delta,
            app.journals.len(),
            area.content.height,
        );
    }
}

// ── Footer click ──────────────────────────────────────────────────────────────

fn footer_click_to_action(
    app: &App,
    mouse: MouseEvent,
    layout: render::TuiLayout,
) -> Option<Action> {
    let hint_id = if app.entry_view_expanded {
        render::expanded_footer_hint_id_at(layout.footer.x, mouse.column)
    } else {
        render::footer_hint_id_at(app, layout.footer.x, mouse.column)
    };

    hint_id.and_then(|id| hint_id_to_action(app, id))
}

// ── Dialog mouse routing ──────────────────────────────────────────────────────

fn handle_overlay_mouse(
    terminal: Option<&mut Terminal<CrosstermBackend<io::Stdout>>>,
    app: &mut App,
    mouse: MouseEvent,
    area: Rect,
) -> AppResult<()> {
    let action = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => overlay_left_click(app, mouse, area),
        MouseEventKind::Drag(MouseButton::Left) => {
            handle_overlay_drag(app, mouse, area);
            None
        }
        MouseEventKind::ScrollUp => {
            handle_overlay_wheel(app, mouse, area, -1);
            None
        }
        MouseEventKind::ScrollDown => {
            handle_overlay_wheel(app, mouse, area, 1);
            None
        }
        _ => None,
    };

    if let Some(action) = action
        && let Some(terminal) = terminal
    {
        super::dispatch_action(terminal, app, action)?;
    }

    Ok(())
}

fn overlay_left_click(app: &mut App, mouse: MouseEvent, area: Rect) -> Option<Action> {
    let col = mouse.column;
    let row = mouse.row;

    if let Some(focus) = app.edit_tag_state().map(|s| s.focus) {
        let filtered_len = app.edit_tag_state().map_or(0, |s| s.filtered.len());
        let layout = render::tags_dialog_layout(area, filtered_len);
        if row == layout.hints.y
            && let Some(id) =
                render::hint_id_at(render::tags_dialog_hints(focus), layout.hints.x + 1, col)
        {
            return hint_id_to_action(app, id);
        }
        if render::point_in_rect(layout.list, col, row) {
            if let Some(state) = app.edit_tag_state_mut() {
                state.focus = crate::tui::state::EditTagFocus::List;
                if let Some(index) =
                    list_row_at(layout.list, col, row, state.offset(), filtered_len)
                {
                    state.select_index(index);
                    state.toggle_selected();
                }
            }
            return None;
        }
        if render::point_in_rect(layout.input, col, row) {
            if let Some(state) = app.edit_tag_state_mut() {
                state.focus = crate::tui::state::EditTagFocus::Input;
            }
            return None;
        }
        return None;
    }

    if app.edit_feeling_state().is_some() {
        let all_len = app.edit_feeling_state().map_or(0, |s| s.all_feelings.len());
        let layout = render::feelings_dialog_layout(area, all_len);
        if row == layout.hints.y
            && let Some(id) =
                render::hint_id_at(render::feelings_dialog_hints(), layout.hints.x + 1, col)
        {
            return hint_id_to_action(app, id);
        }
        if render::point_in_rect(layout.list, col, row)
            && let Some(state) = app.edit_feeling_state_mut()
            && let Some(index) = list_row_at(layout.list, col, row, state.offset(), all_len)
        {
            state.select_index(index);
            state.toggle_selected();
            return None;
        }
        return None;
    }

    if app.edit_mood_state().is_some() {
        let layout = render::mood_dialog_layout(area);
        if row == layout.hints.y
            && let Some(id) =
                render::hint_id_at(render::mood_dialog_hints(), layout.hints.x + 1, col)
        {
            return hint_id_to_action(app, id);
        }
        if render::point_in_rect(layout.bar, col, row)
            && let Some(state) = app.edit_mood_state_mut()
        {
            state.draft = mood_score_at(layout.bar, col);
        }
    }

    None
}

fn handle_overlay_drag(app: &mut App, mouse: MouseEvent, area: Rect) {
    if app.edit_mood_state().is_none() {
        return;
    }

    let layout = render::mood_dialog_layout(area);
    if render::point_in_rect(layout.bar, mouse.column, mouse.row)
        && let Some(state) = app.edit_mood_state_mut()
    {
        state.draft = mood_score_at(layout.bar, mouse.column);
    }
}

fn handle_overlay_wheel(app: &mut App, mouse: MouseEvent, area: Rect, delta: i16) {
    if app.edit_tag_state().is_some() {
        let filtered_len = app.edit_tag_state().map_or(0, |s| s.filtered.len());
        let layout = render::tags_dialog_layout(area, filtered_len);
        if render::point_in_rect(layout.list, mouse.column, mouse.row)
            && let Some(state) = app.edit_tag_state_mut()
        {
            state.scroll_by(delta, layout.list.height);
        }
        return;
    }

    if app.edit_feeling_state().is_some() {
        let all_len = app.edit_feeling_state().map_or(0, |s| s.all_feelings.len());
        let layout = render::feelings_dialog_layout(area, all_len);
        if render::point_in_rect(layout.list, mouse.column, mouse.row)
            && let Some(state) = app.edit_feeling_state_mut()
        {
            state.scroll_by(delta, layout.list.height);
        }
    }
}

fn list_row_at(list: Rect, _col: u16, row: u16, offset: usize, len: usize) -> Option<usize> {
    let relative_row = row.checked_sub(list.y)? as usize;
    if relative_row >= list.height as usize {
        return None;
    }
    let index = offset.saturating_add(relative_row);
    (index < len).then_some(index)
}

fn mood_score_at(bar: Rect, column: u16) -> i8 {
    if bar.width <= 1 {
        return 0;
    }

    let relative = column.saturating_sub(bar.x).min(bar.width - 1);
    let scaled = (relative as f32 / (bar.width - 1) as f32 * 10.0).round() as i8;
    (scaled - 5).clamp(-5, 5)
}

/// Pure: maps a typed hint id to an Action.
pub(super) fn hint_id_to_action(app: &App, id: render::HintId) -> Option<Action> {
    match id {
        render::HintId::NewJournal => Some(Action::NewJournal),
        render::HintId::NewEntry => Some(Action::NewEntry),
        render::HintId::Refresh => Some(Action::Refresh),
        render::HintId::BeginSearch => Some(Action::BeginSearch),
        render::HintId::Quit => Some(Action::Quit),
        render::HintId::EditSelected if app.can_act_on_selected_entry() => {
            Some(Action::EditSelected)
        }
        render::HintId::ViewSelected if app.has_selected_entry_target() => {
            Some(Action::ViewSelected)
        }
        render::HintId::BeginDelete if app.has_selected_entry_target() => Some(Action::BeginDelete),
        render::HintId::BeginEditTags if app.has_selected_entry_target() => {
            Some(Action::BeginEditTags)
        }
        render::HintId::BeginEditFeelings if app.has_selected_entry_target() => {
            Some(Action::BeginEditFeelings)
        }
        render::HintId::BeginEditMood if app.has_selected_entry_target() => {
            Some(Action::BeginEditMood)
        }
        render::HintId::ExitSearch => Some(Action::ExitSearch),
        render::HintId::CancelOverlay => Some(Action::CancelOverlay),
        render::HintId::TagsToggle
            if app
                .edit_tag_state()
                .is_some_and(|state| !state.filtered.is_empty()) =>
        {
            Some(Action::TagsToggle)
        }
        render::HintId::TagsSwitchFocus => Some(Action::TagsSwitchFocus),
        render::HintId::TagsAddFromInput => Some(Action::TagsAddFromInput),
        render::HintId::TagsSave => Some(Action::TagsSave),
        render::HintId::FeelingsToggle => Some(Action::FeelingsToggle),
        render::HintId::FeelingsSave => Some(Action::FeelingsSave),
        render::HintId::MoodDecrease => Some(Action::MoodDecrease),
        render::HintId::MoodIncrease => Some(Action::MoodIncrease),
        render::HintId::MoodSave => Some(Action::MoodSave),
        render::HintId::MoodClear => Some(Action::MoodClear),
        _ => None,
    }
}
