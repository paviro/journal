use crate::AppResult;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, ScrollbarDrag, inline_entry_view_is_visible, single_panel_is_active},
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
        let footer = footer_area(app, area);
        if render::point_in_rect(footer, mouse.column, mouse.row) {
            if let Some(action) = footer_click_to_action(app, mouse, footer) {
                return super::dispatch_action(terminal, app, action);
            }
            return Ok(false);
        }
        // Clicking an entry-view `[Image N …]` label opens the viewer via the
        // same action as the footer hint and keyboard shortcut.
        if let Some(index) = app.image_label_at(mouse.column, mouse.row) {
            return super::dispatch_action(terminal, app, Action::OpenImageViewer(index));
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

    let layout = render::tui_layout(area, app);

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if !try_scrollbar_press(app, mouse, &layout) {
                handle_left_click(app, mouse, layout)?;
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => handle_scrollbar_drag(app, mouse, &layout),
        MouseEventKind::Up(MouseButton::Left) => app.scrollbar_drag = None,
        MouseEventKind::ScrollUp => handle_wheel(app, mouse, layout, -1),
        MouseEventKind::ScrollDown => handle_wheel(app, mouse, layout, 1),
        _ => {}
    }

    Ok(())
}

/// Scroll ranges for each pane that has a draggable scrollbar. Computed from the
/// live layout and caches so a drag always maps against the current content.
struct ScrollbarTarget {
    which: ScrollbarDrag,
    bar: Rect,
    max_scroll: usize,
}

/// The scrollbar target under the cursor's row, if the cursor is on a pane's
/// scrollbar column and that pane actually overflows (so a bar is drawn).
fn scrollbar_target_at(
    app: &mut App,
    column: u16,
    row: u16,
    layout: &render::TuiLayout,
) -> Option<ScrollbarTarget> {
    // Entry view first: its bar sits on the same right edge as the others but the
    // pane is widest, so test it before the narrower list panels.
    if let Some(area) = layout.entry_view {
        let hits = &app.entry_view_image_hits;
        let max = hits
            .line_count
            .saturating_sub(hits.content_rect.height as usize);
        if let Some(bar) = scrollbar_hit(area.area, column, row, max) {
            return Some(ScrollbarTarget {
                which: ScrollbarDrag::EntryView,
                bar,
                max_scroll: max,
            });
        }
    }

    if let Some(area) = layout.entries {
        let cache = app.entry_rows(area.text_width);
        let max = cache
            .total_height
            .saturating_sub(area.viewport_height as usize);
        if let Some(bar) = scrollbar_hit(area.panel.area, column, row, max) {
            return Some(ScrollbarTarget {
                which: ScrollbarDrag::EntryList,
                bar,
                max_scroll: max,
            });
        }
    }

    if app.mode == Mode::Browse
        && let Some(area) = layout.journals
    {
        let per_page = render::journals_per_page(render::journal_list_rect(area.content).height);
        let max = app.journals.len().saturating_sub(per_page as usize);
        if let Some(bar) = scrollbar_hit(area.area, column, row, max) {
            return Some(ScrollbarTarget {
                which: ScrollbarDrag::Journals,
                bar,
                max_scroll: max,
            });
        }
    }

    None
}

/// The scrollbar track rect for `area` if `(column, row)` lands on it and the pane
/// overflows (`max_scroll > 0`, matching `render_scrollbar_if_needed`'s draw guard).
fn scrollbar_hit(area: Rect, column: u16, row: u16, max_scroll: usize) -> Option<Rect> {
    if max_scroll == 0 {
        return None;
    }
    let bar = crate::tui::scroll::scrollbar_bar_rect(area);
    (bar.height > 0 && column == bar.x && row >= bar.y && row < bar.y + bar.height).then_some(bar)
}

/// Apply a scroll offset mapped from `row` on `target`'s track to the right pane.
fn apply_scrollbar_scroll(app: &mut App, target: &ScrollbarTarget, row: u16) {
    let offset =
        crate::tui::scroll::scroll_from_bar_row(row, target.bar.y, target.bar.height, target.max_scroll);
    match target.which {
        ScrollbarDrag::Journals => {
            *app.journal_list.offset_mut() = offset;
            app.focus = Focus::Journals;
        }
        ScrollbarDrag::EntryList => {
            *app.entry_list.offset_mut() = offset;
            app.focus = Focus::Entries;
        }
        ScrollbarDrag::EntryView => {
            app.scroll.entry_view = offset.min(u16::MAX as usize) as u16;
            app.focus = Focus::EntryView;
        }
    }
}

/// On a left press, if the cursor is on a pane's scrollbar, begin dragging it and
/// jump the thumb to the pressed position. Returns whether a bar was grabbed.
fn try_scrollbar_press(app: &mut App, mouse: MouseEvent, layout: &render::TuiLayout) -> bool {
    let Some(target) = scrollbar_target_at(app, mouse.column, mouse.row, layout) else {
        return false;
    };
    app.scrollbar_drag = Some(target.which);
    apply_scrollbar_scroll(app, &target, mouse.row);
    true
}

/// While a scrollbar drag is active, map the cursor row to a scroll offset. The
/// cursor column is ignored so the drag survives drifting off the one-column bar.
fn handle_scrollbar_drag(app: &mut App, mouse: MouseEvent, layout: &render::TuiLayout) {
    let Some(which) = app.scrollbar_drag else {
        return;
    };
    let bar = match which {
        ScrollbarDrag::Journals => layout.journals.map(|a| (a.area, journals_max(app, a))),
        ScrollbarDrag::EntryList => layout.entries.map(|a| {
            let cache = app.entry_rows(a.text_width);
            (
                a.panel.area,
                cache
                    .total_height
                    .saturating_sub(a.viewport_height as usize),
            )
        }),
        ScrollbarDrag::EntryView => layout.entry_view.map(|a| {
            let hits = &app.entry_view_image_hits;
            (
                a.area,
                hits.line_count
                    .saturating_sub(hits.content_rect.height as usize),
            )
        }),
    };
    if let Some((area, max_scroll)) = bar {
        let target = ScrollbarTarget {
            which,
            bar: crate::tui::scroll::scrollbar_bar_rect(area),
            max_scroll,
        };
        apply_scrollbar_scroll(app, &target, mouse.row);
    }
}

fn journals_max(app: &App, area: render::PanelGeometry) -> usize {
    let per_page = render::journals_per_page(render::journal_list_rect(area.content).height);
    app.journals.len().saturating_sub(per_page as usize)
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
        if let Some(index) = journal_box_at(
            area.content,
            mouse.row,
            app.journal_list.offset(),
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
        let cache = app.entry_rows(area.text_width);
        if let Some(index) = render::entry_index_at(
            area,
            mouse.column,
            mouse.row,
            app.entry_list.offset(),
            &cache.meta,
        ) {
            app.select_entry_index(index);
            if !inline_entry_view_is_visible(layout.content.width) {
                view_selected(app)?;
            }
        } else if app.mode == Mode::Browse {
            // Clicking empty space in the list deselects, revealing journal stats.
            app.selected_entry_index = None;
        }
        return Ok(());
    }

    if let Some(area) = layout.entry_view
        && render::point_in_rect(area.area, mouse.column, mouse.row)
        && app.has_selected_entry_target()
    {
        let tags = app.selected_entry_tags();
        let people = app.selected_entry_people();
        let activities = app.selected_entry_activities();
        let feelings = app.selected_entry_feelings();
        let mood = app.selected_entry_mood();
        let metadata = render::EntryMetadataValues {
            tags: &tags,
            people: &people,
            activities: &activities,
            feelings: &feelings,
            mood,
        };
        if let Some(feeling) =
            render::feeling_at_point(area.area, mouse.column, mouse.row, metadata)
        {
            app.begin_feeling_search(&feeling);
            return Ok(());
        }
        if let Some(person) = render::person_at_point(area.area, mouse.column, mouse.row, metadata)
        {
            app.begin_people_search(&person);
            return Ok(());
        }
        if let Some(activity) =
            render::activity_at_point(area.area, mouse.column, mouse.row, metadata)
        {
            app.begin_activity_search(&activity);
            return Ok(());
        }
        if let Some(tag) = render::tag_at_point(area.area, mouse.column, mouse.row, metadata) {
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
        let cache = app.entry_rows(area.text_width);
        app.entry_list_scroll(delta, cache.total_height, area.viewport_height);
        return;
    }

    if app.mode == Mode::Browse
        && let Some(area) = layout.journals
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        app.journal_list_scroll(
            delta,
            render::journals_per_page(render::journal_list_rect(area.content).height),
        );
    }
}

// ── Footer click ──────────────────────────────────────────────────────────────

fn footer_click_to_action(app: &App, mouse: MouseEvent, footer: Rect) -> Option<Action> {
    let hint_id = if single_panel_is_active(footer.width) && app.focus == Focus::EntryView {
        render::expanded_footer_hint_id_at_point(
            app,
            footer.x,
            footer.y,
            footer.width,
            mouse.column,
            mouse.row,
        )
    } else {
        render::footer_hint_id_at_point(
            app,
            footer.x,
            footer.y,
            footer.width,
            mouse.column,
            mouse.row,
        )
    };

    hint_id.and_then(|id| hint_id_to_action(app, id))
}

fn footer_area(app: &App, area: Rect) -> Rect {
    if single_panel_is_active(area.width) && app.focus == Focus::EntryView {
        let height = render::expanded_footer_height(app, area.width).min(area.height);
        return Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(height),
            width: area.width,
            height,
        };
    }

    render::tui_layout(area, app).footer
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

    if let Some((focus, input_is_empty)) = app
        .edit_tag_state()
        .map(|s| (s.focus, s.input.trim().is_empty()))
    {
        let filtered_len = app.edit_tag_state().map_or(0, |s| s.filtered.len());
        let layout = render::tags_dialog_layout(area, filtered_len);
        if render::point_in_rect(layout.hints, col, row)
            && let Some(id) = render::hint_id_at_wrapped(
                render::tags_dialog_hints(focus, input_is_empty),
                layout.hints.x + 1,
                layout.hints.y,
                layout.hints.width.saturating_sub(1),
                col,
                row,
            )
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
        if render::point_in_rect(layout.hints, col, row)
            && let Some(id) = render::hint_id_at_wrapped(
                render::feelings_dialog_hints(),
                layout.hints.x + 1,
                layout.hints.y,
                layout.hints.width.saturating_sub(1),
                col,
                row,
            )
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
        if render::point_in_rect(layout.hints, col, row)
            && let Some(id) = render::hint_id_at_wrapped(
                render::mood_dialog_hints(),
                layout.hints.x + 1,
                layout.hints.y,
                layout.hints.width.saturating_sub(1),
                col,
                row,
            )
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

/// Maps a click row inside the journal panel content to a journal index,
/// accounting for the leading offset and each journal's multi-row bordered box.
fn journal_box_at(content: Rect, row: u16, offset: usize, len: usize) -> Option<usize> {
    let list = render::journal_list_rect(content);
    let relative_row = row.checked_sub(list.y)?;
    if relative_row >= list.height {
        return None;
    }
    let index = offset.saturating_add((relative_row / render::JOURNAL_BOX_HEIGHT) as usize);
    (index < len).then_some(index)
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
        render::HintId::BeginEditPeople if app.has_selected_entry_target() => {
            Some(Action::BeginEditPeople)
        }
        render::HintId::BeginEditActivities if app.has_selected_entry_target() => {
            Some(Action::BeginEditActivities)
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
        render::HintId::OpenImageViewer if app.selected_entry_image_count() > 0 => {
            Some(Action::OpenImageViewer(0))
        }
        render::HintId::HintsToggle => Some(Action::ToggleHints),
        render::HintId::ToggleJournals => Some(Action::ToggleJournals),
        _ => None,
    }
}
