use crate::AppResult;
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{App, Focus, Mode, ScrollbarDrag, inline_entry_view_is_visible},
    render,
    state::ListNav,
};

use super::action::Action;
use super::actions::view_selected;

pub(crate) fn handle_mouse(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
) -> AppResult<bool> {
    let area = super::terminal_area(terminal)?;

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
        MouseEventKind::Up(MouseButton::Left) => app.scrollbar.active = None,
        MouseEventKind::ScrollUp => handle_wheel(app, mouse, layout, -1),
        MouseEventKind::ScrollDown => handle_wheel(app, mouse, layout, 1),
        _ => {}
    }

    Ok(())
}

/// True for the two wheel event kinds.
pub(crate) fn is_wheel(kind: MouseEventKind) -> bool {
    matches!(kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown)
}

/// One line per notch: up scrolls toward the top (negative), down toward the
/// bottom (positive). Non-wheel kinds contribute nothing.
fn wheel_delta(kind: MouseEventKind) -> i16 {
    match kind {
        MouseEventKind::ScrollUp => -1,
        MouseEventKind::ScrollDown => 1,
        _ => 0,
    }
}

/// Sum the deltas of the leading run of wheel events, returning the net movement
/// and how many events were consumed. Stops at the first non-wheel event so its
/// handling isn't skipped. Used to collapse a macOS smooth-scroll burst into one
/// applied step.
pub(crate) fn fold_leading_wheel(events: &[Event]) -> (i16, usize) {
    let mut net = 0;
    let mut count = 0;
    for event in events {
        match event {
            Event::Mouse(m) if is_wheel(m.kind) => {
                net += wheel_delta(m.kind);
                count += 1;
            }
            _ => break,
        }
    }
    (net, count)
}

/// Apply a coalesced wheel delta at `mouse`'s position. Mirrors the non-overlay
/// wheel path of `handle_mouse_in_area`; the caller guarantees no overlay is open.
pub(crate) fn handle_scroll(app: &mut App, mouse: MouseEvent, area: Rect, net_delta: i16) {
    let layout = render::tui_layout(area, app);
    handle_wheel(app, mouse, layout, net_delta);
}

/// A pane's scrollbar geometry, resolved from the live layout and caches so both a
/// press and an ongoing drag map against the current content.
struct ScrollbarTarget {
    which: ScrollbarDrag,
    /// The full bar column (`scrollbar_bar_rect`); its first/last rows are the arrows.
    bar: Rect,
    max_scroll: usize,
    content_length: usize,
    viewport: u16,
    /// Current scrollbar position, for locating the thumb.
    position: usize,
}

impl ScrollbarTarget {
    /// The thumb's `(top, len)` rows, replicating ratatui so it matches what's drawn.
    fn thumb(&self) -> (u16, u16) {
        crate::tui::scroll::scrollbar_thumb(
            self.bar,
            self.content_length,
            self.viewport,
            self.position,
        )
        .unwrap_or((self.bar.y.saturating_add(1), 1))
    }
}

/// Resolve a pane's scrollbar geometry, or `None` when the pane is absent or does not
/// overflow (so no bar is drawn — matching `render_scrollbar_if_needed`'s guard).
fn pane_target(
    app: &App,
    which: ScrollbarDrag,
    layout: &render::TuiLayout,
) -> Option<ScrollbarTarget> {
    let (area, content_length, viewport, scroll) = match which {
        ScrollbarDrag::EntryView => {
            let area = layout.entry_view?;
            let hits = &app.entry_view_image_hits;
            (
                area.area,
                hits.line_count,
                hits.content_rect.height,
                app.nav.scroll.entry_view as usize,
            )
        }
        ScrollbarDrag::EntryList => {
            let area = layout.entries?;
            let cache = app.entry_rows(area.text_width);
            (
                area.panel.area,
                cache.total_height,
                area.viewport_height,
                app.nav.entry_list.offset(),
            )
        }
        ScrollbarDrag::Journals => {
            if app.nav.mode != Mode::Browse {
                return None;
            }
            let area = layout.journals?;
            let (_, meta, list_area) = app.journal_rows(area.content);
            let total_height = crate::tui::entry_rows::total_row_height(&meta);
            (
                area.area,
                total_height,
                list_area.height,
                app.nav.journal_list.offset(),
            )
        }
        ScrollbarDrag::Insights => {
            // The list tabs record their geometry at render time; other tabs leave
            // `total == 0`, so no bar is offered.
            let geometry = &app.insights_scroll;
            (
                geometry.area,
                geometry.total,
                geometry.viewport,
                app.nav.scroll.insights as usize,
            )
        }
    };
    let max_scroll = content_length.saturating_sub(viewport as usize);
    let bar = crate::tui::scroll::scrollbar_bar_rect(area);
    if max_scroll == 0 || bar.height == 0 {
        return None;
    }
    let position = crate::tui::scroll::scrollbar_position(scroll, content_length, viewport);
    Some(ScrollbarTarget {
        which,
        bar,
        max_scroll,
        content_length,
        viewport,
        position,
    })
}

/// The scrollbar target under the cursor, if any. Panes are probed independently;
/// their bars sit on distinct panel edges, so at most one contains the cursor.
fn scrollbar_target_at(
    app: &App,
    column: u16,
    row: u16,
    layout: &render::TuiLayout,
) -> Option<ScrollbarTarget> {
    [
        ScrollbarDrag::EntryView,
        ScrollbarDrag::EntryList,
        ScrollbarDrag::Journals,
        ScrollbarDrag::Insights,
    ]
    .into_iter()
    .filter_map(|which| pane_target(app, which, layout))
    .find(|target| cursor_on_bar(target, column, row))
}

/// Whether `(column, row)` lands on `target`'s grab region — the bar column plus one
/// on each side, so the one-cell bar is easier to hit. The bar sits on the panel's
/// right border, so the right-neighbour column is the adjacent panel's left edge;
/// that pane's own bar is on its far side and never claims this column back, so a
/// click there just scrolls this pane.
fn cursor_on_bar(target: &ScrollbarTarget, column: u16, row: u16) -> bool {
    let bar = target.bar;
    let on_column = column >= bar.x.saturating_sub(1) && column <= bar.x.saturating_add(1);
    on_column && row >= bar.y && row < bar.y + bar.height
}

/// Set a pane's scroll offset directly (already clamped to its `max_scroll`).
fn set_pane_scroll(app: &mut App, which: ScrollbarDrag, offset: usize) {
    match which {
        ScrollbarDrag::Journals => {
            *app.nav.journal_list.offset_mut() = offset;
            app.focus_journals_from_click(false);
        }
        ScrollbarDrag::EntryList => {
            *app.nav.entry_list.offset_mut() = offset;
            app.focus_entries();
        }
        ScrollbarDrag::EntryView => {
            app.nav.scroll.entry_view = offset.min(u16::MAX as usize) as u16;
            app.focus_entry_view_from_click();
        }
        ScrollbarDrag::Insights => {
            app.nav.scroll.insights = offset.min(u16::MAX as usize) as u16;
            app.focus_insights();
        }
    }
}

/// Step a pane's scroll by one line, reusing the same setters the wheel uses.
fn step_pane_scroll(app: &mut App, target: &ScrollbarTarget, delta: i16) {
    match target.which {
        ScrollbarDrag::Journals => {
            app.scroll_journal_list(delta, target.content_length, target.viewport);
            app.focus_journals_from_click(false);
        }
        ScrollbarDrag::EntryList => {
            app.scroll_entry_list(delta, target.content_length, target.viewport);
            app.focus_entries();
        }
        ScrollbarDrag::EntryView => {
            app.scroll_entry_view(delta);
            app.focus_entry_view_from_click();
        }
        ScrollbarDrag::Insights => {
            app.scroll_insights(delta);
            app.focus_insights();
        }
    }
}

/// Map the dragged cursor row to a scroll offset so the grabbed point of the thumb
/// (`scrollbar.grab` rows below its top) tracks the cursor. The cursor column is
/// ignored, so the drag survives drifting off the narrow bar.
fn apply_thumb_drag(app: &mut App, target: &ScrollbarTarget, row: u16) {
    let (_, thumb_len) = target.thumb();
    let track_top = target.bar.y.saturating_add(1);
    let track_len = target.bar.height.saturating_sub(2);
    let thumb_top = row.saturating_sub(app.scrollbar.grab);
    let offset = crate::tui::scroll::scroll_from_thumb_top(
        thumb_top,
        track_top,
        track_len,
        thumb_len,
        target.max_scroll,
    );
    set_pane_scroll(app, target.which, offset);
}

/// On a left press over a pane's scrollbar: the arrow rows step by one line; pressing
/// the thumb grabs it without moving; pressing empty track jumps the thumb under the
/// cursor. Returns whether the press was consumed.
fn try_scrollbar_press(app: &mut App, mouse: MouseEvent, layout: &render::TuiLayout) -> bool {
    let Some(target) = scrollbar_target_at(app, mouse.column, mouse.row, layout) else {
        return false;
    };
    let bar = target.bar;

    // Top / bottom arrow rows step one line, like the wheel, rather than jumping.
    if mouse.row == bar.y {
        step_pane_scroll(app, &target, -1);
        return true;
    }
    if mouse.row == bar.y + bar.height - 1 {
        step_pane_scroll(app, &target, 1);
        return true;
    }

    let (thumb_top, thumb_len) = target.thumb();
    app.scrollbar.active = Some(target.which);
    if mouse.row >= thumb_top && mouse.row < thumb_top + thumb_len {
        // Grabbing the thumb itself: remember where, and leave the scroll untouched so
        // a click straight on the handle doesn't jump.
        app.scrollbar.grab = mouse.row - thumb_top;
    } else {
        // Empty track: centre the thumb on the cursor and jump there.
        app.scrollbar.grab = thumb_len / 2;
        apply_thumb_drag(app, &target, mouse.row);
    }
    true
}

/// While a scrollbar drag is active, map the cursor row to the pane's scroll offset.
fn handle_scrollbar_drag(app: &mut App, mouse: MouseEvent, layout: &render::TuiLayout) {
    let Some(which) = app.scrollbar.active else {
        return;
    };
    if let Some(target) = pane_target(app, which, layout) {
        apply_thumb_drag(app, &target, mouse.row);
    }
}

fn handle_left_click(app: &mut App, mouse: MouseEvent, layout: render::TuiLayout) -> AppResult<()> {
    if app.nav.mode == Mode::Browse
        && let Some(area) = layout.journals
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        app.focus_journals_from_click(layout.single_panel);
        let (_, meta, _) = app.journal_rows(area.content);
        if let Some(index) = render::journal_index_at(
            area.content,
            mouse.column,
            mouse.row,
            app.nav.journal_list.offset(),
            &meta,
        ) {
            app.select_journal(index);
        }
        return Ok(());
    }

    if let Some(area) = layout.entries
        && render::point_in_rect(area.panel.area, mouse.column, mouse.row)
    {
        app.focus_entries();
        let cache = app.entry_rows(area.text_width);
        if let Some(index) = render::entry_index_at(
            area,
            mouse.column,
            mouse.row,
            app.nav.entry_list.offset(),
            &cache.meta,
        ) {
            app.select_entry_index(index);
            if !inline_entry_view_is_visible(layout.content.width) {
                view_selected(app)?;
            }
        } else if app.nav.mode == Mode::Browse {
            // Clicking empty space in the list deselects, revealing journal insights.
            app.clear_entry_selection();
        }
        return Ok(());
    }

    if let Some(area) = layout.insights
        && render::point_in_rect(area.area, mouse.column, mouse.row)
        && app.nav.mode == Mode::Browse
    {
        app.focus_insights();
        // Clicking a tab in the border selects it; clicking elsewhere just focuses.
        if let Some(tab) = render::insights_tab_at(area.area, mouse.column, mouse.row) {
            app.select_insights_tab(tab);
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
            // Location is display-only — not part of the click hit-test.
            location: &[],
        };
        if let Some((chip, value)) =
            render::metadata_at_point(area.area, mouse.column, mouse.row, metadata)
        {
            match chip {
                render::MetadataChip::Feelings => app.begin_feeling_search(&value),
                render::MetadataChip::People => app.begin_people_search(&value),
                render::MetadataChip::Activities => app.begin_activity_search(&value),
                render::MetadataChip::Tags => app.begin_tag_search(&value),
            }
            return Ok(());
        }
        // Focus the viewer on the pane. A click already inside a full-screen viewer
        // must not collapse it, so `focus_entry_view_from_click` only resets fullscreen
        // when focus enters from another column.
        app.focus_entry_view_from_click();
    }

    Ok(())
}

fn handle_wheel(app: &mut App, mouse: MouseEvent, layout: render::TuiLayout, delta: i16) {
    // Probed first, and via the rendered geometry rather than a layout slot, so it
    // also catches the insights shown in the preview column when no entry is selected.
    if app.insights_scroll.total > 0
        && render::point_in_rect(app.insights_scroll.area, mouse.column, mouse.row)
    {
        app.scroll_insights(delta);
        return;
    }

    if let Some(area) = layout.entry_view
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        app.scroll_entry_view(delta);
        return;
    }

    if let Some(area) = layout.entries
        && render::point_in_rect(area.panel.area, mouse.column, mouse.row)
    {
        let cache = app.entry_rows(area.text_width);
        app.scroll_entry_list(delta, cache.total_height, area.viewport_height);
        return;
    }

    if app.nav.mode == Mode::Browse
        && let Some(area) = layout.journals
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        let (_, meta, list_area) = app.journal_rows(area.content);
        let total_height = crate::tui::entry_rows::total_row_height(&meta);
        app.scroll_journal_list(delta, total_height, list_area.height);
    }
}

// ── Footer click ──────────────────────────────────────────────────────────────

fn footer_click_to_action(app: &App, mouse: MouseEvent, footer: Rect) -> Option<Action> {
    let hint_id = if app.entry_view_is_fullscreen(footer.width) {
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
    if app.entry_view_is_fullscreen(area.width) {
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

/// Route a click landing on a dialog's hint bar to its action, if any.
fn dialog_hint_action(
    app: &App,
    hints_area: Rect,
    hints: &[render::Hint],
    col: u16,
    row: u16,
) -> Option<Action> {
    if !render::point_in_rect(hints_area, col, row) {
        return None;
    }
    let id = render::hint_id_at_wrapped(
        hints,
        hints_area.x + 1,
        hints_area.y,
        hints_area.width.saturating_sub(1),
        col,
        row,
    )?;
    hint_id_to_action(app, id)
}

fn overlay_left_click(app: &mut App, mouse: MouseEvent, area: Rect) -> Option<Action> {
    let col = mouse.column;
    let row = mouse.row;

    if let Some((focus, input_is_empty)) = app
        .edit_metadata_state()
        .map(|s| (s.focus, s.input.trim().is_empty()))
    {
        let filtered_len = app.edit_metadata_state().map_or(0, |s| s.filtered.len());
        let layout = render::metadata_dialog_layout(area, filtered_len);
        if let Some(action) = dialog_hint_action(
            app,
            layout.hints,
            render::metadata_dialog_hints(focus, input_is_empty),
            col,
            row,
        ) {
            return Some(action);
        }
        if render::point_in_rect(layout.list, col, row) {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.focus = crate::tui::state::EditMetadataFocus::List;
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
            if let Some(state) = app.edit_metadata_state_mut() {
                state.focus = crate::tui::state::EditMetadataFocus::Input;
            }
            return None;
        }
        return None;
    }

    if let Some(focus) = app.edit_feeling_state().map(|s| s.focus) {
        let (all_len, selected_lines) = app.edit_feeling_state().map_or((0, 1), |s| {
            (
                s.item_count(),
                render::feelings_selected_line_count(&s.selected),
            )
        });
        let layout = render::feelings_dialog_layout(area, all_len, selected_lines);
        if let Some(action) = dialog_hint_action(
            app,
            layout.hints,
            render::feelings_dialog_hints(focus),
            col,
            row,
        ) {
            return Some(action);
        }
        if render::point_in_rect(layout.list, col, row) {
            if let Some(state) = app.edit_feeling_state_mut() {
                state.focus = crate::tui::state::EditMetadataFocus::List;
                if let Some(index) = list_row_at(layout.list, col, row, state.offset(), all_len)
                    && index < state.item_count()
                {
                    // Clicking a header folds it; clicking a feeling toggles it.
                    state.select_index(index);
                    state.toggle_selected();
                }
            }
            return None;
        }
        if render::point_in_rect(layout.input, col, row) {
            if let Some(state) = app.edit_feeling_state_mut() {
                state.focus = crate::tui::state::EditMetadataFocus::Input;
            }
            return None;
        }
        return None;
    }

    if app.edit_mood_state().is_some() {
        let layout = render::mood_dialog_layout(area);
        if let Some(action) =
            dialog_hint_action(app, layout.hints, render::mood_dialog_hints(), col, row)
        {
            return Some(action);
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
    if app.edit_metadata_state().is_some() {
        let filtered_len = app.edit_metadata_state().map_or(0, |s| s.filtered.len());
        let layout = render::metadata_dialog_layout(area, filtered_len);
        if render::point_in_rect(layout.list, mouse.column, mouse.row)
            && let Some(state) = app.edit_metadata_state_mut()
        {
            state.scroll_by(delta, layout.list.height);
        }
        return;
    }

    if app.edit_feeling_state().is_some() {
        let (all_len, selected_lines) = app.edit_feeling_state().map_or((0, 1), |s| {
            (
                s.item_count(),
                render::feelings_selected_line_count(&s.selected),
            )
        });
        let layout = render::feelings_dialog_layout(area, all_len, selected_lines);
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
        render::HintId::ToggleArchiveJournal
            if app.nav.focus == Focus::Journals && app.selected_journal().is_some() =>
        {
            Some(Action::ToggleArchiveJournal)
        }
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
        // In multi-column full screen the flag is set, so collapse back to the pane;
        // otherwise (single-column) exit the viewer to the entries list.
        render::HintId::CloseEntryView => Some(if app.nav.entry_view_fullscreen {
            Action::CollapseEntryView
        } else {
            Action::FocusLeft
        }),
        render::HintId::MetadataToggle
            if app
                .edit_metadata_state()
                .is_some_and(|state| !state.filtered.is_empty()) =>
        {
            Some(Action::MetadataToggle)
        }
        render::HintId::MetadataSwitchFocus => Some(Action::MetadataSwitchFocus),
        render::HintId::MetadataAddFromInput => Some(Action::MetadataAddFromInput),
        render::HintId::MetadataSave => Some(Action::MetadataSave),
        render::HintId::FeelingsToggle => Some(Action::FeelingsToggle),
        render::HintId::FeelingsExpand => Some(Action::FeelingsExpand),
        render::HintId::FeelingsCollapse => Some(Action::FeelingsCollapse),
        render::HintId::FeelingsSwitchFocus => Some(Action::FeelingsSwitchFocus),
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
        // Clicking the tabs hint steps forward through the tabs (Right); scope
        // toggles — both only while the insights panel is focused.
        render::HintId::InsightsTab if app.insights_panel_focused() => Some(Action::FocusRight),
        render::HintId::InsightsScope if app.insights_panel_focused() => {
            Some(Action::ToggleInsightsScope)
        }
        render::HintId::InsightsTimeframe if app.insights_panel_focused() => {
            Some(Action::CycleInsightsTimeframe)
        }
        render::HintId::ExpandInsights if app.insights_panel_focused() => {
            Some(Action::ExpandInsights)
        }
        render::HintId::CloseInsights if app.insights_panel_focused() => {
            Some(Action::CollapseInsights)
        }
        _ => None,
    }
}
