use crate::AppResult;
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;

use crate::tui::{
    app::{
        App, EditLocationFocus, EditMetadataFocus, Focus, Mode, ScrollbarDrag,
        inline_reader_is_visible,
    },
    editor_state::EditorPrompt,
    render,
    state::{HoverTarget, ListNav, MetadataKind, Overlay},
};

use super::DispatchOutcome;
use super::action::{Action, InsightsAction, ReaderAction};
use super::actions::view_selected;

fn editor_mouse_action(app: &App, mouse: MouseEvent) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::ScrollDown => Some(Action::EditorScroll(1)),
        MouseEventKind::ScrollUp => Some(Action::EditorScroll(-1)),
        MouseEventKind::Down(MouseButton::Left) => Some(Action::EditorStartSelection {
            col: mouse.column,
            row: mouse.row,
        }),
        MouseEventKind::Drag(MouseButton::Left) if app.editor.as_ref()?.mouse_selecting => {
            Some(Action::EditorDragSelection {
                col: mouse.column,
                row: mouse.row,
            })
        }
        MouseEventKind::Up(MouseButton::Left) => Some(Action::EditorEndSelection),
        _ => None,
    }
}

fn editor_prompt_is_open(app: &App) -> bool {
    !matches!(
        app.editor.as_ref().map(|editor| &editor.prompt),
        None | Some(EditorPrompt::None)
    )
}

fn editor_prompt_mouse_action(app: &App, mouse: MouseEvent, area: Rect) -> Option<Action> {
    let prompt = app.editor.as_ref().map(|editor| &editor.prompt)?;
    match prompt {
        EditorPrompt::None => None,
        EditorPrompt::ConfirmDiscard => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                render::editor_discard_choice_at_point(area, mouse.column, mouse.row).map(
                    |discard| {
                        if discard {
                            Action::EditorDiscard
                        } else {
                            Action::EditorClosePrompt
                        }
                    },
                )
            }
            _ => None,
        },
        EditorPrompt::MetadataMenu => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let mode = render::MetadataMenuMode::Editor;
                if render::metadata_menu_close_at_point(area, mode, mouse.column, mouse.row) {
                    return Some(Action::EditorClosePrompt);
                }
                match render::metadata_menu_choice_at_point(area, mode, mouse.column, mouse.row) {
                    Some(render::MetadataChoice::Metadata(kind)) => {
                        Some(Action::BeginEditMetadata(kind))
                    }
                    Some(render::MetadataChoice::Feelings) => Some(Action::BeginEditFeelings),
                    Some(render::MetadataChoice::Mood) => Some(Action::BeginEditMood),
                    Some(render::MetadataChoice::Location) => Some(Action::BeginEditLocation),
                    None => None,
                }
            }
            _ => None,
        },
        EditorPrompt::Help { scroll } => match mouse.kind {
            MouseEventKind::ScrollDown => Some(Action::EditorScrollHelp(1)),
            MouseEventKind::ScrollUp => Some(Action::EditorScrollHelp(-1)),
            MouseEventKind::Down(MouseButton::Left) => {
                if render::editor_shortcut_close_at_point(area, *scroll, mouse.column, mouse.row) {
                    return Some(Action::EditorClosePrompt);
                }
                let id =
                    render::editor_shortcut_hint_at_point(area, *scroll, mouse.column, mouse.row)?;
                hint_id_to_action(app, id)
            }
            _ => None,
        },
    }
}

pub(crate) fn handle_mouse(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
) -> AppResult<DispatchOutcome> {
    let area = super::terminal_area(terminal)?;
    super::dispatch_action(terminal, app, Action::PointerInput { event: mouse, area })
}

pub(super) fn apply_pointer(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
    area: Rect,
) -> AppResult<DispatchOutcome> {
    if try_toast_dismiss(app, mouse, area) {
        return Ok(DispatchOutcome::Continue);
    }

    if app.has_overlay() {
        handle_overlay_mouse(Some(terminal), app, mouse, area)?;
        return Ok(DispatchOutcome::Continue);
    }

    if let Some(action) = editor_prompt_mouse_action(app, mouse, area) {
        return super::dispatch_action(terminal, app, action);
    } else if editor_prompt_is_open(app) {
        return Ok(DispatchOutcome::Continue);
    }

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        let footer = footer_area(app, area);
        if render::point_in_rect(footer, mouse.column, mouse.row) {
            if let Some(action) = footer_click_to_action(app, mouse, footer) {
                return super::dispatch_action(terminal, app, action);
            }
            return Ok(DispatchOutcome::Continue);
        }
        // Clicking an entry-view `[Image N …]` label opens the viewer via the
        // same action as the footer hint and keyboard shortcut.
        if let Some(index) = app.image_label_at(mouse.column, mouse.row) {
            return super::dispatch_action(terminal, app, Action::OpenImageViewer(index));
        }
        if let Some(target) = app.reader_link_at(mouse.column, mouse.row) {
            return super::dispatch_action(terminal, app, Action::OpenReaderLink(target));
        }
    }

    if app.editor.is_some() {
        if let Some(action) = editor_mouse_action(app, mouse) {
            return super::dispatch_action(terminal, app, action);
        }
        return Ok(DispatchOutcome::Continue);
    }

    apply_pointer_in_area(app, mouse, area)?;
    Ok(DispatchOutcome::Continue)
}

/// A left press on a toast dismisses it. Probed before everything else —
/// toasts render topmost, so they must win the hit-test.
fn try_toast_dismiss(app: &mut App, mouse: MouseEvent, area: Rect) -> bool {
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return false;
    }
    let Some(index) = render::toast_at_point(app, area, mouse.column, mouse.row) else {
        return false;
    };
    app.toasts.dismiss(index);
    app.hover = HoverTarget::None;
    true
}

pub(super) fn apply_pointer_in_area(app: &mut App, mouse: MouseEvent, area: Rect) -> AppResult<()> {
    if try_toast_dismiss(app, mouse, area) {
        return Ok(());
    }
    if app.has_overlay() {
        handle_overlay_mouse(None, app, mouse, area)?;
        return Ok(());
    }
    if handle_text_field_mouse(app, mouse) {
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
/// wheel path of `apply_pointer_in_area`; the caller guarantees no overlay is open.
pub(crate) fn handle_scroll(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mouse: MouseEvent,
    area: Rect,
    net_delta: i16,
) -> AppResult<()> {
    super::dispatch_action(
        terminal,
        app,
        Action::PointerScroll {
            event: mouse,
            area,
            delta: net_delta,
        },
    )?;
    Ok(())
}

pub(super) fn apply_scroll(app: &mut App, mouse: MouseEvent, area: Rect, net_delta: i16) {
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
        ScrollbarDrag::Reader => {
            let area = layout.reader?;
            let hits = &app.reader_image_hits;
            (
                area.area,
                hits.line_count,
                hits.content_rect.height,
                app.nav.scroll.reader as usize,
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
        ScrollbarDrag::Reader,
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
        ScrollbarDrag::Reader => {
            app.nav.scroll.reader = offset.min(u16::MAX as usize) as u16;
            app.focus_reader_from_click();
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
        ScrollbarDrag::Reader => {
            app.scroll_reader(delta);
            app.focus_reader_from_click();
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
            if !inline_reader_is_visible(layout.content.width) {
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

    if let Some(area) = layout.reader
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
            location: None,
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
        // must not collapse it, so `focus_reader_from_click` only resets fullscreen
        // when focus enters from another column.
        app.focus_reader_from_click();
    }

    Ok(())
}

/// Pixel-row lists (entry list, journal column) scroll this many rows per
/// wheel notch: their items are 3-5 rows tall, so the 1-row step that suits
/// line-granular panes reads as a crawl there.
const WHEEL_PIXELS_PER_NOTCH: i16 = 2;

fn handle_wheel(app: &mut App, mouse: MouseEvent, layout: render::TuiLayout, delta: i16) {
    // Probed first, and via the rendered geometry rather than a layout slot, so it
    // also catches the insights shown in the reader column when no entry is selected.
    if app.insights_scroll.total > 0
        && render::point_in_rect(app.insights_scroll.area, mouse.column, mouse.row)
    {
        app.focus_insights();
        app.scroll_insights(delta);
        return;
    }

    if let Some(area) = layout.reader
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        app.focus_reader_from_click();
        app.scroll_reader(delta);
        return;
    }

    if let Some(area) = layout.entries
        && render::point_in_rect(area.panel.area, mouse.column, mouse.row)
    {
        let cache = app.entry_rows(area.text_width);
        app.scroll_entry_list(
            delta.saturating_mul(WHEEL_PIXELS_PER_NOTCH),
            cache.total_height,
            area.viewport_height,
        );
        return;
    }

    if app.nav.mode == Mode::Browse
        && let Some(area) = layout.journals
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        let (_, meta, list_area) = app.journal_rows(area.content);
        let total_height = crate::tui::entry_rows::total_row_height(&meta);
        app.scroll_journal_list(
            delta.saturating_mul(WHEEL_PIXELS_PER_NOTCH),
            total_height,
            list_area.height,
        );
    }
}

// ── Hover ─────────────────────────────────────────────────────────────────────

/// Track what's under the cursor. Returns whether the hover target changed —
/// the run loop only repaints then, so motion inside one row costs nothing.
/// Hovering never moves a selection — not in the main panels (selecting has
/// side effects: journal switch, reader swap) and not in dialogs (the theme
/// picker previews on click, not hover). It only highlights the row under the
/// cursor.
pub(crate) fn update_hover(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    col: u16,
    row: u16,
    area: Rect,
) -> AppResult<bool> {
    let previous = app.hover;
    super::dispatch_action(
        terminal,
        app,
        Action::PointerHover {
            column: col,
            row,
            area,
        },
    )?;
    Ok(previous != app.hover)
}

pub(super) fn apply_hover(app: &mut App, col: u16, row: u16, area: Rect) -> bool {
    let target = hover_target_at(app, col, row, area);
    if target == app.hover {
        return false;
    }
    app.hover = target;
    true
}

/// The hover target under `(col, row)`, probed in the click paths' priority
/// order and through the same geometry helpers, so hover and click can never
/// disagree about what's under the cursor.
fn hover_target_at(app: &App, col: u16, row: u16, area: Rect) -> HoverTarget {
    // Toasts render topmost, so they win the probe like they win clicks.
    if let Some(index) = render::toast_at_point(app, area, col, row) {
        return HoverTarget::Toast(index);
    }

    if app.has_overlay() {
        return overlay_hover_target(app, col, row, area);
    }

    if editor_prompt_is_open(app) {
        return editor_prompt_hover_target(app, col, row, area);
    }

    // The footer first: it overlays nothing, so it can't shadow a list row.
    // Covers the editor's footer too — its hints are the same clickable kind.
    let footer = footer_area(app, area);
    if render::point_in_rect(footer, col, row) {
        if let Some(id) = footer_hint_at(app, footer, col, row) {
            return HoverTarget::FooterHint(id);
        }
        return HoverTarget::None;
    }

    if app.editor.is_some() {
        return HoverTarget::None;
    }

    // The search query field rides the entries panel's border, so it wins
    // over the panel probe below.
    if let Some(target) = text_field_hover_at(app, col, row) {
        return target;
    }

    // Reader links and image labels, matching the click path's priority
    // (labels before links). Both self-bound-check the reader's content rect.
    if let Some(line) = app.reader_image_line_at(col, row) {
        return HoverTarget::ReaderImage(line);
    }
    if let Some((line, start, end)) = app.reader_link_hit_at(col, row) {
        return HoverTarget::ReaderLink { line, start, end };
    }

    let layout = render::tui_layout(area, app);
    if let Some(panel) = layout.insights
        && render::point_in_rect(panel.area, col, row)
        && let Some(tab) = render::insights_tab_at(panel.area, col, row)
    {
        return HoverTarget::InsightsTab(tab);
    }

    if app.nav.mode == Mode::Browse
        && let Some(panel) = layout.journals
        && render::point_in_rect(panel.area, col, row)
    {
        let (_, meta, _) = app.journal_rows(panel.content);
        if let Some(index) = render::journal_index_at(
            panel.content,
            col,
            row,
            app.nav.journal_list.offset(),
            &meta,
        ) {
            return HoverTarget::Journal(index);
        }
        return HoverTarget::None;
    }

    if let Some(geometry) = layout.entries
        && render::point_in_rect(geometry.panel.area, col, row)
        && let Some(index) = render::entry_index_at(
            geometry,
            col,
            row,
            app.nav.entry_list.offset(),
            &app.entry_rows(geometry.text_width).meta,
        )
    {
        return HoverTarget::Entry(index);
    }

    HoverTarget::None
}

/// The text field under `(col, row)`, identified by its last-drawn rect —
/// the read-only sibling of [`focus_text_field_at`], probing the same fields.
fn text_field_hover_at(app: &App, col: u16, row: u16) -> Option<HoverTarget> {
    let field = |input: &crate::tui::text_input::TextInput| {
        input
            .hit_col(col, row)
            .map(|_| HoverTarget::TextField(input.last_area()))
    };
    match &app.overlay {
        Overlay::NewJournal(input) => field(input),
        Overlay::EditMetadata(state) => field(&state.input),
        Overlay::EditFeelings(state) => field(&state.input),
        Overlay::EditLocation(state) => field(&state.query).or_else(|| field(&state.name)),
        Overlay::None if app.nav.mode == Mode::Search => field(&app.search.query),
        _ => None,
    }
}

/// The hover target inside the open overlay, mirroring [`overlay_left_click`]'s
/// per-dialog geometry: list/menu rows, confirm buttons, and hint chips.
fn overlay_hover_target(app: &App, col: u16, row: u16, area: Rect) -> HoverTarget {
    if let Some(target) = text_field_hover_at(app, col, row) {
        return target;
    }
    let hint = |hints_area: Rect, hints: &[render::Hint]| -> Option<HoverTarget> {
        render::point_in_rect(hints_area, col, row)
            .then(|| {
                render::hint_id_at_wrapped(
                    hints,
                    hints_area.x + 1,
                    hints_area.y,
                    hints_area.width.saturating_sub(1),
                    col,
                    row,
                )
            })
            .flatten()
            .map(HoverTarget::FooterHint)
    };

    match &app.overlay {
        Overlay::SettingsMenu => {
            if let Some(index) = render::settings_menu_row_at_point(area, col, row) {
                return HoverTarget::DialogRow(index);
            }
        }
        Overlay::MetadataMenu => {
            if let Some(index) =
                render::metadata_menu_row_at_point(area, render::MetadataMenuMode::Viewer, col, row)
            {
                return HoverTarget::DialogRow(index);
            }
        }
        Overlay::ConfirmDelete(ctx) => {
            let inner = render::confirm_delete_inner(area, ctx);
            if let Some(yes) = render::confirm_button_at(inner, col, row) {
                return HoverTarget::ConfirmButton(yes);
            }
        }
        _ => {}
    }

    if let Some(state) = app.theme_picker_state() {
        let layout =
            render::theme_picker_layout(area, state.entries.len(), state.mode_switchable());
        if render::point_in_rect(layout.list, col, row)
            && let Some(index) =
                list_row_at(layout.list, col, row, state.offset(), state.entries.len())
        {
            return HoverTarget::DialogRow(index);
        }
        return hint(
            layout.hints,
            &render::theme_picker_hints(state.mode_switchable()),
        )
        .unwrap_or_default();
    }

    if let Some(state) = app.edit_metadata_state() {
        let layout = render::metadata_dialog_layout(area, state.filtered.len());
        if render::point_in_rect(layout.list, col, row)
            && let Some(index) =
                list_row_at(layout.list, col, row, state.offset(), state.filtered.len())
        {
            return HoverTarget::DialogRow(index);
        }
        let hints =
            render::metadata_dialog_hints(state.focus, state.input.as_str().trim().is_empty());
        return hint(layout.hints, hints).unwrap_or_default();
    }

    if let Some(state) = app.edit_feeling_state() {
        let layout = render::feelings_dialog_layout(area, state.item_count(), &state.selected);
        if render::point_in_rect(layout.list, col, row)
            && let Some(index) =
                list_row_at(layout.list, col, row, state.offset(), state.item_count())
        {
            return HoverTarget::DialogRow(index);
        }
        return hint(layout.hints, render::feelings_dialog_hints(state.focus)).unwrap_or_default();
    }

    if let Some(state) = app.edit_location_state() {
        let labels = state.list_labels();
        let layout = render::location_dialog_layout(area, &labels);
        if render::point_in_rect(layout.list, col, row)
            && let Some(index) =
                render::location_list_row_at(layout.list, &labels, state.offset(), row)
        {
            return HoverTarget::DialogRow(index);
        }
        let hints = render::location_dialog_hints(state.focus, state.query_looked_up);
        return hint(layout.hints, hints).unwrap_or_default();
    }

    if app.edit_mood_state().is_some() {
        let layout = render::mood_dialog_layout(area);
        return hint(layout.hints, render::mood_dialog_hints()).unwrap_or_default();
    }

    HoverTarget::None
}

/// The hover target inside an open editor prompt (they float like overlays but
/// live on the editor, not `app.overlay`).
fn editor_prompt_hover_target(app: &App, col: u16, row: u16, area: Rect) -> HoverTarget {
    match app.editor.as_ref().map(|editor| &editor.prompt) {
        Some(EditorPrompt::ConfirmDiscard) => {
            match render::editor_discard_choice_at_point(area, col, row) {
                Some(yes) => HoverTarget::ConfirmButton(yes),
                None => HoverTarget::None,
            }
        }
        Some(EditorPrompt::MetadataMenu) => {
            match render::metadata_menu_row_at_point(
                area,
                render::MetadataMenuMode::Editor,
                col,
                row,
            ) {
                Some(index) => HoverTarget::DialogRow(index),
                None => HoverTarget::None,
            }
        }
        _ => HoverTarget::None,
    }
}

// ── Footer click ──────────────────────────────────────────────────────────────

/// The footer hint under `(col, row)`, in whichever footer form is showing.
fn footer_hint_at(app: &App, footer: Rect, col: u16, row: u16) -> Option<render::HintId> {
    if app.reader_is_fullscreen(footer.width) {
        render::expanded_footer_hint_id_at_point(app, footer.x, footer.y, footer.width, col, row)
    } else {
        render::footer_hint_id_at_point(app, footer.x, footer.y, footer.width, col, row)
    }
}

fn footer_click_to_action(app: &App, mouse: MouseEvent, footer: Rect) -> Option<Action> {
    footer_hint_at(app, footer, mouse.column, mouse.row).and_then(|id| hint_id_to_action(app, id))
}

fn footer_area(app: &App, area: Rect) -> Rect {
    if app.reader_is_fullscreen(area.width) {
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
    if handle_text_field_mouse(app, mouse) {
        return Ok(());
    }
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

/// Mouse editing for the single-line text fields (the search box and dialog
/// inputs): a press in a field focuses it, places the caret, and arms a
/// selection; a drag extends it; release finishes it. Returns whether the
/// event was consumed by a field, mirroring the editor's selection flow.
fn handle_text_field_mouse(app: &mut App, mouse: MouseEvent) -> bool {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(col) = focus_text_field_at(app, mouse.column, mouse.row) else {
                return false;
            };
            if let Some(input) = app.focused_text_input_mut() {
                input.begin_mouse_selection(col);
                app.nav.input_selecting = true;
            }
            true
        }
        MouseEventKind::Drag(MouseButton::Left) if app.nav.input_selecting => {
            if let Some(input) = app.focused_text_input_mut() {
                let rect = input.last_area();
                let col = mouse
                    .column
                    .clamp(rect.x, rect.x + rect.width.saturating_sub(1))
                    - rect.x;
                input.drag_mouse_selection(col);
            }
            true
        }
        MouseEventKind::Up(MouseButton::Left) if app.nav.input_selecting => {
            app.nav.input_selecting = false;
            if let Some(input) = app.focused_text_input_mut() {
                input.end_mouse_selection();
            }
            true
        }
        _ => false,
    }
}

/// The text field under `(col, row)`, if any: focuses it and returns the click
/// column within the field.
fn focus_text_field_at(app: &mut App, col: u16, row: u16) -> Option<u16> {
    match &mut app.overlay {
        Overlay::NewJournal(input) => input.hit_col(col, row),
        Overlay::EditMetadata(state) => {
            let hit = state.input.hit_col(col, row)?;
            state.focus = EditMetadataFocus::Input;
            Some(hit)
        }
        Overlay::EditFeelings(state) => {
            let hit = state.input.hit_col(col, row)?;
            state.focus = EditMetadataFocus::Input;
            Some(hit)
        }
        Overlay::EditLocation(state) => {
            if let Some(hit) = state.query.hit_col(col, row) {
                state.focus = EditLocationFocus::Query;
                Some(hit)
            } else if let Some(hit) = state.name.hit_col(col, row) {
                state.focus = EditLocationFocus::Name;
                Some(hit)
            } else {
                None
            }
        }
        Overlay::None if app.nav.mode == Mode::Search => {
            let hit = app.search.query.hit_col(col, row)?;
            app.nav.focus = Focus::Entries;
            Some(hit)
        }
        _ => None,
    }
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

    if matches!(app.overlay, Overlay::MetadataMenu) {
        let mode = render::MetadataMenuMode::Viewer;
        if render::metadata_menu_close_at_point(area, mode, col, row) {
            return Some(Action::CancelOverlay);
        }
        return match render::metadata_menu_choice_at_point(area, mode, col, row)? {
            render::MetadataChoice::Metadata(MetadataKind::Tags) => {
                Some(Action::BeginEditMetadata(MetadataKind::Tags))
            }
            render::MetadataChoice::Metadata(MetadataKind::People) => {
                Some(Action::BeginEditMetadata(MetadataKind::People))
            }
            render::MetadataChoice::Metadata(MetadataKind::Activities) => {
                Some(Action::BeginEditMetadata(MetadataKind::Activities))
            }
            render::MetadataChoice::Feelings => Some(Action::BeginEditFeelings),
            render::MetadataChoice::Mood => Some(Action::BeginEditMood),
            render::MetadataChoice::Location => Some(Action::BeginEditLocation),
        };
    }

    if matches!(app.overlay, Overlay::SettingsMenu) {
        if render::settings_menu_close_at_point(area, col, row) {
            return Some(Action::CancelOverlay);
        }
        return match render::settings_menu_choice_at_point(area, col, row)? {
            render::SettingsChoice::Theme => Some(Action::OpenThemePicker),
        };
    }

    if let Some(state) = app.theme_picker_state() {
        let len = state.entries.len();
        let offset = state.offset();
        let mode_switchable = state.mode_switchable();
        let layout = render::theme_picker_layout(area, len, mode_switchable);
        if let Some(action) = dialog_hint_action(
            app,
            layout.hints,
            &render::theme_picker_hints(mode_switchable),
            col,
            row,
        ) {
            return Some(action);
        }
        if render::point_in_rect(layout.list, col, row)
            && let Some(index) = list_row_at(layout.list, col, row, offset, len)
        {
            return Some(Action::ThemePickerSelect(index));
        }
        return None;
    }

    if let Some((focus, input_is_empty)) = app
        .edit_metadata_state()
        .map(|s| (s.focus, s.input.as_str().trim().is_empty()))
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
                state.focus = EditMetadataFocus::List;
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
                state.focus = EditMetadataFocus::Input;
            }
            return None;
        }
        return None;
    }

    if let Some(focus) = app.edit_feeling_state().map(|s| s.focus) {
        let layout = app.edit_feeling_state().map_or_else(
            || render::feelings_dialog_layout(area, 0, &[]),
            |state| render::feelings_dialog_layout(area, state.item_count(), &state.selected),
        );
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
                state.focus = EditMetadataFocus::List;
                if let Some(index) =
                    list_row_at(layout.list, col, row, state.offset(), state.item_count())
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
                state.focus = EditMetadataFocus::Input;
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
        return None;
    }

    if let Some((focus, query_looked_up)) = app
        .edit_location_state()
        .map(|s| (s.focus, s.query_looked_up))
    {
        let labels = app
            .edit_location_state()
            .map_or_else(Vec::new, |state| state.list_labels());
        let layout = render::location_dialog_layout(area, &labels);
        if let Some(action) = dialog_hint_action(
            app,
            layout.hints,
            render::location_dialog_hints(focus, query_looked_up),
            col,
            row,
        ) {
            return Some(action);
        }
        if render::point_in_rect(layout.query, col, row) {
            if let Some(state) = app.edit_location_state_mut() {
                state.focus = EditLocationFocus::Query;
            }
            return None;
        }
        if render::point_in_rect(layout.name, col, row) {
            if let Some(state) = app.edit_location_state_mut() {
                state.focus = EditLocationFocus::Name;
            }
            return None;
        }
        if render::point_in_rect(layout.list, col, row) {
            let offset = app.edit_location_state().map_or(0, |s| s.offset());
            let index = render::location_list_row_at(layout.list, &labels, offset, row);
            if let Some(state) = app.edit_location_state_mut() {
                state.focus = EditLocationFocus::List;
                if let Some(index) = index {
                    state.select_index(index);
                    return Some(Action::LocationSelectRow);
                }
            }
            return None;
        }
        return None;
    }

    if let Overlay::ConfirmDelete(ctx) = &app.overlay {
        let inner = render::confirm_delete_inner(area, ctx);
        return match render::confirm_button_at(inner, col, row) {
            Some(true) => Some(Action::ConfirmDelete),
            Some(false) => Some(Action::CancelOverlay),
            None => None,
        };
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
        let layout = app.edit_feeling_state().map_or_else(
            || render::feelings_dialog_layout(area, 0, &[]),
            |state| render::feelings_dialog_layout(area, state.item_count(), &state.selected),
        );
        if render::point_in_rect(layout.list, mouse.column, mouse.row)
            && let Some(state) = app.edit_feeling_state_mut()
        {
            state.scroll_by(delta, layout.list.height);
        }
        return;
    }

    if let Some(labels) = app.edit_location_state().map(|s| s.list_labels()) {
        let layout = render::location_dialog_layout(area, &labels);
        if render::point_in_rect(layout.list, mouse.column, mouse.row)
            && let Some(state) = app.edit_location_state_mut()
        {
            state.scroll_by(delta, layout.list.height);
        }
        return;
    }

    if let Some((len, mode_switchable)) = app
        .theme_picker_state()
        .map(|s| (s.entries.len(), s.mode_switchable()))
    {
        let layout = render::theme_picker_layout(area, len, mode_switchable);
        if render::point_in_rect(layout.list, mouse.column, mouse.row)
            && let Some(state) = app.theme_picker_state_mut()
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
        render::HintId::InputSelectAll => Some(Action::InputSelectAll),
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
        render::HintId::ExitSearch => Some(Action::ExitSearch),
        render::HintId::CancelOverlay => Some(Action::CancelOverlay),
        // In multi-column full screen the flag is set, so collapse back to the pane;
        // otherwise (single-column) exit the viewer to the entries list.
        render::HintId::CloseReader => Some(if app.nav.reader_fullscreen {
            Action::Reader(ReaderAction::SetFullscreen(false))
        } else {
            Action::FocusLeft
        }),
        // The focused-viewer "enter" chip expands to full screen, matching the key.
        render::HintId::ExpandReader if app.nav.focus == Focus::Reader => {
            Some(Action::Reader(ReaderAction::SetFullscreen(true)))
        }
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
        render::HintId::MoodDecrease => Some(Action::AdjustMood(-1)),
        render::HintId::MoodIncrease => Some(Action::AdjustMood(1)),
        render::HintId::MoodSave => Some(Action::MoodSave),
        render::HintId::MoodClear => Some(Action::MoodClear),
        render::HintId::LocationSwitchFocus => Some(Action::LocationSwitchFocus),
        render::HintId::LocationResolve => Some(Action::LocationResolve),
        render::HintId::LocationGrabDevice => Some(Action::LocationGrabDevice),
        render::HintId::LocationSelectRow => Some(Action::LocationSelectRow),
        render::HintId::LocationSave => Some(Action::LocationSave),
        render::HintId::LocationClear => Some(Action::LocationClear),
        render::HintId::OpenImageViewer if app.selected_entry_image_count() > 0 => {
            Some(Action::OpenImageViewer(0))
        }
        render::HintId::OpenMetadataMenu if app.can_act_on_selected_entry() => {
            Some(Action::OpenMetadataMenu)
        }
        render::HintId::OpenSettings => Some(Action::OpenSettingsMenu),
        render::HintId::ThemePickerApply => Some(Action::ThemePickerConfirm),
        render::HintId::ThemePickerRevert => Some(Action::ThemePickerCancel),
        render::HintId::ThemePickerChrome => Some(Action::ThemePickerCycleChrome),
        render::HintId::ThemePickerMode => Some(Action::ThemePickerCycleMode),
        render::HintId::HintsToggle => Some(Action::ToggleHints),
        render::HintId::ToggleJournals => Some(Action::ToggleJournals),
        // Clicking the tabs hint steps forward through the tabs (Right); scope
        // toggles — both only while the insights panel is focused.
        render::HintId::InsightsTab if app.insights_panel_focused() => Some(Action::FocusRight),
        render::HintId::InsightsScope if app.insights_panel_focused() => {
            Some(Action::Insights(InsightsAction::ToggleScope))
        }
        render::HintId::InsightsTimeframe if app.insights_panel_focused() => {
            Some(Action::Insights(InsightsAction::CycleTimeframe))
        }
        render::HintId::ExpandInsights if app.insights_panel_focused() => {
            Some(Action::Insights(InsightsAction::SetFullscreen(true)))
        }
        render::HintId::CloseInsights if app.insights_panel_focused() => {
            Some(Action::Insights(InsightsAction::SetFullscreen(false)))
        }
        render::HintId::EditorSave => Some(Action::EditorSave),
        render::HintId::EditorDiscard => Some(Action::EditorRequestDiscard),
        render::HintId::EditorFullscreen => Some(Action::EditorToggleFullscreen),
        render::HintId::EditorMetadata => Some(Action::EditorOpenMetadataMenu),
        render::HintId::EditorHelp => Some(Action::EditorOpenHelp),
        _ => None,
    }
}
