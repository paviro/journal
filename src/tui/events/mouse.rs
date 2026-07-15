use crate::AppResult;
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    layout::Rect,
};
use std::io;

use crate::tui::{
    app::{AppModel, Focus, Mode, ScrollbarDrag, inline_reader_is_visible},
    editor_state::EditorPrompt,
    features::{location::EditLocationFocus, metadata::EditMetadataFocus},
    render,
    state::{HoverTarget, ListNav, MetadataKind, Overlay},
    ui::{ConfirmId, DialogId, DialogInputId, InteractionKind, ViewState, interaction::PanelId},
};

use super::DispatchOutcome;
use super::action::{
    Action, BrowserAction, DialogListTarget, EditMetadataFocusTarget, EditorAction, ImageAction,
    InsightsAction, LocationAction, MetadataAction, MetadataSearchTarget, MouseAction,
    OverlayAction, ScrollbarMetrics, SearchAction, SettingsAction, TextFieldTarget,
};

mod overlay;
use overlay::{
    footer_area, footer_click_to_action, footer_hint_at, mapped_hover_target, overlay_mouse_action,
    prompt_mouse_action, text_field_hover_at, text_field_mouse_action,
};

fn editor_mouse_action(app: &AppModel, mouse: MouseEvent, double_click: bool) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::ScrollDown => Some(Action::Editor(EditorAction::Scroll(1))),
        MouseEventKind::ScrollUp => Some(Action::Editor(EditorAction::Scroll(-1))),
        MouseEventKind::Down(MouseButton::Left) if double_click => {
            Some(Action::Editor(EditorAction::SelectWord {
                col: mouse.column,
                row: mouse.row,
            }))
        }
        MouseEventKind::Down(MouseButton::Left) => {
            Some(Action::Editor(EditorAction::StartSelection {
                col: mouse.column,
                row: mouse.row,
            }))
        }
        MouseEventKind::Drag(MouseButton::Left) if app.editor.as_ref()?.mouse_selecting => {
            Some(Action::Editor(EditorAction::DragSelection {
                col: mouse.column,
                row: mouse.row,
            }))
        }
        MouseEventKind::Up(MouseButton::Left) => Some(Action::Editor(EditorAction::EndSelection)),
        _ => None,
    }
}

fn editor_prompt_is_open(app: &AppModel) -> bool {
    !matches!(
        app.editor.as_ref().map(|editor| &editor.prompt),
        None | Some(EditorPrompt::None)
    )
}

pub(crate) fn handle_mouse(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppModel,
    mouse: MouseEvent,
    view: &ViewState,
) -> AppResult<DispatchOutcome> {
    let area = super::terminal_area(terminal)?;
    let double_click = mouse.kind == MouseEventKind::Down(MouseButton::Left)
        && app.nav.register_left_click(mouse.column, mouse.row);
    let Some(action) = mouse_to_action(app, mouse, area, view, double_click) else {
        return Ok(DispatchOutcome::Continue);
    };
    super::dispatch_action(terminal, app, action)
}

pub(super) fn mouse_to_action(
    app: &AppModel,
    mouse: MouseEvent,
    area: Rect,
    view: &ViewState,
    double_click: bool,
) -> Option<Action> {
    if mouse.kind == MouseEventKind::Down(MouseButton::Left)
        && let Some(index) = render::toast_at_point(app, area, mouse.column, mouse.row)
    {
        return Some(Action::Mouse(MouseAction::DismissToast(index)));
    }

    if app.has_overlay() {
        return overlay_mouse_action(app, mouse, area, view, double_click);
    }

    if let Some(action) = prompt_mouse_action(app, mouse, view) {
        return Some(action);
    } else if editor_prompt_is_open(app) {
        return None;
    }

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        match view.interactions.hit(mouse.column, mouse.row) {
            Some(InteractionKind::Hint(id)) => return hint_id_to_action(app, *id),
            Some(InteractionKind::Image(index)) => {
                return Some(Action::Images(ImageAction::OpenViewer(*index)));
            }
            Some(InteractionKind::Link {
                target,
                heading_line,
            }) => {
                return Some(Action::Browser(BrowserAction::OpenReaderLink {
                    target: target.clone(),
                    heading_line: *heading_line,
                }));
            }
            _ => {}
        }
        let footer = footer_area(app, area);
        if render::point_in_rect(footer, mouse.column, mouse.row) {
            if let Some(action) = footer_click_to_action(app, mouse, footer) {
                return Some(action);
            }
            return None;
        }
    }

    if app.editor.is_some() {
        if let Some(action) = editor_mouse_action(app, mouse, double_click) {
            return Some(action);
        }
        return None;
    }

    if let Some(action) = text_field_mouse_action(app, mouse, view, double_click) {
        return Some(Action::Mouse(action));
    }

    let layout = view.layout.unwrap_or_else(|| render::tui_layout(area, app));
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            match view.interactions.hit(mouse.column, mouse.row) {
                Some(InteractionKind::Scrollbar(metrics)) => {
                    Some(Action::Mouse(MouseAction::ScrollbarPress {
                        metrics: *metrics,
                        row: mouse.row,
                    }))
                }
                Some(InteractionKind::Row { panel, index }) => {
                    panel_click_action(app, *panel, Some(*index), &layout, mouse)
                }
                Some(InteractionKind::Panel(panel)) => {
                    panel_click_action(app, *panel, None, &layout, mouse)
                }
                _ => None,
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => app.scrollbar.active.and_then(|which| {
            view.interactions.scrollbar(which).map(|metrics| {
                Action::Mouse(MouseAction::ScrollbarDrag {
                    metrics,
                    row: mouse.row,
                })
            })
        }),
        MouseEventKind::Up(MouseButton::Left) => Some(Action::Mouse(MouseAction::ScrollbarRelease)),
        MouseEventKind::ScrollUp => wheel_action(app, mouse, layout, -1, view),
        MouseEventKind::ScrollDown => wheel_action(app, mouse, layout, 1, view),
        _ => None,
    }
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
    app: &mut AppModel,
    mouse: MouseEvent,
    area: Rect,
    net_delta: i16,
    view: &ViewState,
) -> AppResult<()> {
    let layout = view.layout.unwrap_or_else(|| render::tui_layout(area, app));
    if let Some(action) = wheel_action(app, mouse, layout, net_delta, view) {
        super::dispatch_action(terminal, app, action)?;
    }
    Ok(())
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

impl From<ScrollbarMetrics> for ScrollbarTarget {
    fn from(metrics: ScrollbarMetrics) -> Self {
        Self {
            which: metrics.which,
            bar: metrics.bar,
            max_scroll: metrics.max_scroll,
            content_length: metrics.content_length,
            viewport: metrics.viewport,
            position: metrics.position,
        }
    }
}

/// Set a pane's scroll offset directly (already clamped to its `max_scroll`).
fn set_pane_scroll(app: &mut AppModel, which: ScrollbarDrag, offset: usize) {
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
fn step_pane_scroll(app: &mut AppModel, target: &ScrollbarTarget, delta: i16) {
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
fn apply_thumb_drag(app: &mut AppModel, target: &ScrollbarTarget, row: u16) {
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

/// Translate a click on a registered panel (or row) region into its action,
/// preserving each panel's mode/selection guards.
fn panel_click_action(
    app: &AppModel,
    panel: PanelId,
    index: Option<usize>,
    layout: &render::TuiLayout,
    mouse: MouseEvent,
) -> Option<Action> {
    match panel {
        PanelId::Journals => {
            (app.nav.mode == Mode::Browse).then_some(Action::Mouse(MouseAction::JournalClick {
                index,
                compact: layout.single_panel,
            }))
        }
        PanelId::Entries => Some(Action::Mouse(MouseAction::EntryClick {
            index,
            open_reader: !inline_reader_is_visible(layout.content.width),
            clear_empty: app.nav.mode == Mode::Browse,
        })),
        PanelId::Insights => {
            if app.nav.mode != Mode::Browse {
                return None;
            }
            let area = layout.insights?;
            // Clicking a tab in the border selects it; clicking elsewhere just focuses.
            let tab =
                render::insights_tab_at(&app.appearance.theme, area.area, mouse.column, mouse.row);
            Some(Action::Mouse(MouseAction::InsightsClick(tab)))
        }
        PanelId::Reader => {
            let area = layout.reader?;
            if !app.has_selected_entry_target() {
                return None;
            }
            let metadata = app.selected_entry_metadata_values();
            if let Some((chip, value)) = render::metadata_at_point(
                &app.appearance.theme,
                area.area,
                mouse.column,
                mouse.row,
                metadata.values(),
            ) {
                let kind = match chip {
                    render::MetadataChip::Feelings => MetadataSearchTarget::Feelings,
                    render::MetadataChip::People => {
                        MetadataSearchTarget::Metadata(MetadataKind::People)
                    }
                    render::MetadataChip::Activities => {
                        MetadataSearchTarget::Metadata(MetadataKind::Activities)
                    }
                    render::MetadataChip::Tags => {
                        MetadataSearchTarget::Metadata(MetadataKind::Tags)
                    }
                };
                return Some(Action::Mouse(MouseAction::MetadataSearch { kind, value }));
            }
            // Focus the viewer on the pane. A click already inside a full-screen viewer
            // must not collapse it, so `focus_reader_from_click` only resets fullscreen
            // when focus enters from another column.
            Some(Action::Mouse(MouseAction::ReaderClick))
        }
    }
}

/// Pixel-row lists (entry list, journal column) scroll this many rows per
/// wheel notch: their items are 3-5 rows tall, so the 1-row step that suits
/// line-granular panes reads as a crawl there.
const WHEEL_PIXELS_PER_NOTCH: i16 = 2;

fn wheel_action(
    app: &AppModel,
    mouse: MouseEvent,
    layout: render::TuiLayout,
    delta: i16,
    view: &ViewState,
) -> Option<Action> {
    // Probed first, and via the rendered geometry rather than a layout slot, so it
    // also catches the insights shown in the reader column when no entry is selected.
    if view.insights.total > 0 && render::point_in_rect(view.insights.area, mouse.column, mouse.row)
    {
        return Some(Action::Mouse(MouseAction::ScrollPanel {
            panel: PanelId::Insights,
            delta,
            content_length: view.insights.total,
            viewport: view.insights.viewport,
        }));
    }

    if let Some(area) = layout.reader
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        return Some(Action::Mouse(MouseAction::ScrollPanel {
            panel: PanelId::Reader,
            delta,
            content_length: view.reader.line_count,
            viewport: view.reader.content_rect.height,
        }));
    }

    if let Some(area) = layout.entries
        && render::point_in_rect(area.panel.area, mouse.column, mouse.row)
    {
        let cache = app.entry_rows(area.text_width);
        return Some(Action::Mouse(MouseAction::ScrollPanel {
            panel: PanelId::Entries,
            delta: delta.saturating_mul(WHEEL_PIXELS_PER_NOTCH),
            content_length: cache.total_height,
            viewport: area.viewport_height,
        }));
    }

    if app.nav.mode == Mode::Browse
        && let Some(area) = layout.journals
        && render::point_in_rect(area.area, mouse.column, mouse.row)
    {
        let (_, meta, list_area) = app.journal_rows(area.content);
        let total_height = crate::tui::entry_rows::total_row_height(&meta);
        return Some(Action::Mouse(MouseAction::ScrollPanel {
            panel: PanelId::Journals,
            delta: delta.saturating_mul(WHEEL_PIXELS_PER_NOTCH),
            content_length: total_height,
            viewport: list_area.height,
        }));
    }
    None
}

pub(super) fn apply_mouse_action(
    app: &mut AppModel,
    action: MouseAction,
) -> AppResult<Option<Action>> {
    match action {
        MouseAction::DismissToast(index) => {
            app.toasts.dismiss(index);
            app.hover = HoverTarget::None;
        }
        MouseAction::TextFieldPress { target, column } => {
            focus_text_field(app, target);
            if let Some(input) = text_field_mut(app, target) {
                input.begin_mouse_selection(column);
                app.nav.input_selecting = true;
            }
        }
        MouseAction::TextFieldSelectWord { target, column } => {
            focus_text_field(app, target);
            if let Some(input) = text_field_mut(app, target) {
                input.select_word_at(column);
            }
        }
        MouseAction::TextFieldDrag { column } => {
            if let Some(input) = app.focused_text_input_mut() {
                input.drag_mouse_selection(column);
            }
        }
        MouseAction::TextFieldRelease => {
            app.nav.input_selecting = false;
            if let Some(input) = app.focused_text_input_mut() {
                input.end_mouse_selection();
            }
        }
        MouseAction::JournalClick { index, compact } => {
            app.focus_journals_from_click(compact);
            if let Some(index) = index {
                app.select_journal(index);
            }
        }
        MouseAction::EntryClick {
            index,
            open_reader,
            clear_empty,
        } => {
            app.focus_entries();
            if let Some(index) = index {
                app.select_entry_index(index);
                if open_reader {
                    return Ok(Some(Action::Browser(BrowserAction::ViewSelected)));
                }
            } else if clear_empty {
                app.clear_entry_selection();
            }
        }
        MouseAction::InsightsClick(tab) => {
            app.focus_insights();
            if let Some(tab) = tab {
                app.select_insights_tab(tab);
            }
        }
        MouseAction::ReaderClick => app.focus_reader_from_click(),
        MouseAction::MetadataSearch { kind, value } => match kind {
            MetadataSearchTarget::Feelings => app.begin_feeling_search(&value),
            MetadataSearchTarget::Metadata(MetadataKind::People) => app.begin_people_search(&value),
            MetadataSearchTarget::Metadata(MetadataKind::Activities) => {
                app.begin_activity_search(&value)
            }
            MetadataSearchTarget::Metadata(MetadataKind::Tags) => app.begin_tag_search(&value),
        },
        MouseAction::ScrollPanel {
            panel,
            delta,
            content_length,
            viewport,
        } => match panel {
            PanelId::Journals => app.scroll_journal_list(delta, content_length, viewport),
            PanelId::Entries => app.scroll_entry_list(delta, content_length, viewport),
            PanelId::Reader => {
                app.focus_reader_from_click();
                app.scroll_reader(delta);
            }
            PanelId::Insights => {
                app.focus_insights();
                app.scroll_insights(delta);
            }
        },
        MouseAction::ScrollbarPress { metrics, row } => {
            apply_scrollbar_press(app, metrics.into(), row);
        }
        MouseAction::ScrollbarDrag { metrics, row } => {
            let target = ScrollbarTarget::from(metrics);
            apply_thumb_drag(app, &target, row);
        }
        MouseAction::ScrollbarRelease => app.scrollbar.active = None,
        MouseAction::DialogRow { target, index } => match target {
            DialogListTarget::Metadata => {
                if let Some(state) = app.edit_metadata_state_mut() {
                    state.focus = EditMetadataFocus::List;
                    state.select_index(index);
                    state.toggle_selected();
                }
            }
            DialogListTarget::Feelings => {
                if let Some(state) = app.edit_feeling_state_mut() {
                    state.focus = EditMetadataFocus::List;
                    state.select_index(index);
                    state.toggle_selected();
                }
            }
            DialogListTarget::Location => {
                if let Some(state) = app.edit_location_state_mut() {
                    state.focus = EditLocationFocus::List;
                    state.select_index(index);
                    return Ok(Some(Action::Location(LocationAction::SelectRow)));
                }
            }
            DialogListTarget::ThemePicker => {
                return Ok(Some(Action::Settings(SettingsAction::ThemePickerSelect(
                    index,
                ))));
            }
        },
        MouseAction::DialogFocusMetadata(focus) => {
            let focus = match focus {
                EditMetadataFocusTarget::List => EditMetadataFocus::List,
                EditMetadataFocusTarget::Input => EditMetadataFocus::Input,
            };
            if let Some(state) = app.edit_metadata_state_mut() {
                state.focus = focus;
            } else if let Some(state) = app.edit_feeling_state_mut() {
                state.focus = focus;
            }
        }
        MouseAction::DialogFocusLocation(focus) => {
            if let Some(state) = app.edit_location_state_mut() {
                state.focus = focus;
            }
        }
        MouseAction::DialogScroll {
            target,
            delta,
            viewport,
        } => match target {
            DialogListTarget::Metadata => {
                if let Some(state) = app.edit_metadata_state_mut() {
                    state.scroll_by(delta, viewport);
                }
            }
            DialogListTarget::Feelings => {
                if let Some(state) = app.edit_feeling_state_mut() {
                    state.scroll_by(delta, viewport);
                }
            }
            DialogListTarget::Location => {
                if let Some(state) = app.edit_location_state_mut() {
                    state.scroll_by(delta, viewport);
                }
            }
            DialogListTarget::ThemePicker => {
                if let Some(state) = app.theme_picker_state_mut() {
                    state.scroll_by(delta, viewport);
                }
            }
        },
        MouseAction::SetMood(score) => {
            if let Some(state) = app.edit_mood_state_mut() {
                state.draft = score;
            }
        }
    }
    Ok(None)
}

fn focus_text_field(app: &mut AppModel, target: TextFieldTarget) {
    match target {
        TextFieldTarget::Search => app.nav.focus = Focus::Entries,
        TextFieldTarget::Metadata => {
            if let Some(state) = app.edit_metadata_state_mut() {
                state.focus = EditMetadataFocus::Input;
            }
        }
        TextFieldTarget::Feelings => {
            if let Some(state) = app.edit_feeling_state_mut() {
                state.focus = EditMetadataFocus::Input;
            }
        }
        TextFieldTarget::LocationQuery => {
            if let Some(state) = app.edit_location_state_mut() {
                state.focus = EditLocationFocus::Query;
            }
        }
        TextFieldTarget::LocationName => {
            if let Some(state) = app.edit_location_state_mut() {
                state.focus = EditLocationFocus::Name;
            }
        }
        TextFieldTarget::NewJournal => {}
    }
}

fn text_field_mut(
    app: &mut AppModel,
    target: TextFieldTarget,
) -> Option<&mut crate::tui::text_input::TextInput> {
    match target {
        TextFieldTarget::Search => Some(&mut app.search.query),
        TextFieldTarget::NewJournal => match &mut app.overlay {
            Overlay::NewJournal(input) => Some(input),
            _ => None,
        },
        TextFieldTarget::Metadata => app.edit_metadata_state_mut().map(|state| &mut state.input),
        TextFieldTarget::Feelings => app.edit_feeling_state_mut().map(|state| &mut state.input),
        TextFieldTarget::LocationQuery => {
            app.edit_location_state_mut().map(|state| &mut state.query)
        }
        TextFieldTarget::LocationName => app.edit_location_state_mut().map(|state| &mut state.name),
    }
}

fn apply_scrollbar_press(app: &mut AppModel, target: ScrollbarTarget, row: u16) {
    let bar = target.bar;
    if row == bar.y {
        step_pane_scroll(app, &target, -1);
        return;
    }
    if row == bar.y + bar.height - 1 {
        step_pane_scroll(app, &target, 1);
        return;
    }

    let (thumb_top, thumb_len) = target.thumb();
    app.scrollbar.active = Some(target.which);
    if row >= thumb_top && row < thumb_top + thumb_len {
        app.scrollbar.grab = row - thumb_top;
    } else {
        app.scrollbar.grab = thumb_len / 2;
        apply_thumb_drag(app, &target, row);
    }
}

// ── Hover ─────────────────────────────────────────────────────────────────────

/// Track what's under the cursor. Returns whether the hover target changed —
/// the run loop only repaints then, so motion inside one row costs nothing.
/// Hovering never moves a selection — not in the main panels (selecting has
/// side effects: journal switch, reader swap) and not in dialogs (the theme
/// picker previews on click, not hover). It only highlights the row under the
/// cursor.
pub(crate) fn update_hover<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppModel,
    col: u16,
    row: u16,
    area: Rect,
    view: &ViewState,
) -> AppResult<bool> {
    let previous = app.hover;
    let action = match view.interactions.hit(col, row) {
        Some(InteractionKind::Row {
            panel: PanelId::Journals,
            index,
        }) => Action::SetHover(HoverTarget::Journal(*index)),
        Some(InteractionKind::Row {
            panel: PanelId::Entries,
            index,
        }) => Action::SetHover(HoverTarget::Entry(*index)),
        _ => Action::SetHover(hover_target_at(app, col, row, area, view)),
    };
    super::dispatch_action(terminal, app, action)?;
    Ok(previous != app.hover)
}

/// The hover target under `(col, row)`, probed in the click paths' priority
/// order and through the same registered regions, so hover and click can never
/// disagree about what's under the cursor.
#[cfg(test)]
pub(super) fn hover_action_at(
    app: &AppModel,
    col: u16,
    row: u16,
    area: Rect,
    view: &ViewState,
) -> Action {
    Action::SetHover(hover_target_at(app, col, row, area, view))
}

fn hover_target_at(
    app: &AppModel,
    col: u16,
    row: u16,
    area: Rect,
    view: &ViewState,
) -> HoverTarget {
    // Toasts render topmost, so they win the probe like they win clicks.
    if let Some(index) = render::toast_at_point(app, area, col, row) {
        return HoverTarget::Toast(index);
    }

    if app.has_overlay() || editor_prompt_is_open(app) {
        return mapped_hover_target(col, row, view);
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
    if let Some(target) = text_field_hover_at(col, row, view) {
        return target;
    }

    // Reader links and image labels, matching the click path's priority
    // (labels before links). Both self-bound-check the reader's content rect.
    if let Some(line) = view.reader_image_line_at(col, row) {
        return HoverTarget::ReaderImage(line);
    }
    if let Some((line, start, end)) = view.reader_link_hit_at(col, row) {
        return HoverTarget::ReaderLink { line, start, end };
    }

    let layout = view.layout.unwrap_or_else(|| render::tui_layout(area, app));
    if let Some(panel) = layout.insights
        && render::point_in_rect(panel.area, col, row)
        && let Some(tab) = render::insights_tab_at(&app.appearance.theme, panel.area, col, row)
    {
        return HoverTarget::InsightsTab(tab);
    }

    // Metadata chips in the pinned reader footer, mirroring the click path so
    // hover and click land on the same pill.
    if let Some(panel) = layout.reader
        && render::point_in_rect(panel.area, col, row)
        && app.has_selected_entry_target()
    {
        let metadata = app.selected_entry_metadata_values();
        if let Some(index) = render::metadata_chip_index_at(
            &app.appearance.theme,
            panel.area,
            col,
            row,
            metadata.values(),
        ) {
            return HoverTarget::MetadataChip(index);
        }
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

/// Map a typed hint id to its Action, applying the same focus/target guards as
/// the keyboard so a footer click and its key can't diverge.
pub(super) fn hint_id_to_action(app: &AppModel, id: render::HintId) -> Option<Action> {
    match id {
        render::HintId::InputSelectAll => Some(Action::Overlay(OverlayAction::InputSelectAll)),
        render::HintId::NewJournal => Some(Action::Settings(SettingsAction::NewJournal)),
        render::HintId::ToggleArchiveJournal
            if app.nav.focus == Focus::Journals && app.selected_journal().is_some() =>
        {
            Some(Action::Settings(SettingsAction::ToggleArchiveJournal))
        }
        render::HintId::NewEntry => Some(Action::Browser(BrowserAction::NewEntry)),
        render::HintId::BeginSearch => Some(Action::Search(SearchAction::Begin)),
        render::HintId::Quit => Some(Action::Quit),
        render::HintId::EditSelected if app.can_act_on_selected_entry() => {
            Some(Action::Browser(BrowserAction::EditSelected))
        }
        render::HintId::BeginDelete if app.has_selected_entry_target() => {
            Some(Action::Browser(BrowserAction::BeginDelete))
        }
        render::HintId::ExitSearch => Some(Action::Search(SearchAction::Exit)),
        render::HintId::CancelOverlay => Some(Action::Overlay(OverlayAction::Cancel)),
        render::HintId::MetadataToggle
            if app
                .edit_metadata_state()
                .is_some_and(|state| !state.filtered.is_empty()) =>
        {
            Some(Action::Metadata(MetadataAction::Toggle))
        }
        render::HintId::MetadataSwitchFocus => Some(Action::Metadata(MetadataAction::SwitchFocus)),
        render::HintId::MetadataAddFromInput => {
            Some(Action::Metadata(MetadataAction::AddFromInput))
        }
        render::HintId::MetadataSave => Some(Action::Metadata(MetadataAction::Save)),
        render::HintId::FeelingsToggle => Some(Action::Metadata(MetadataAction::FeelingsToggle)),
        render::HintId::FeelingsExpand => Some(Action::Metadata(MetadataAction::FeelingsExpand)),
        render::HintId::FeelingsCollapse => {
            Some(Action::Metadata(MetadataAction::FeelingsCollapse))
        }
        render::HintId::FeelingsSwitchFocus => {
            Some(Action::Metadata(MetadataAction::FeelingsSwitchFocus))
        }
        render::HintId::FeelingsSave => Some(Action::Metadata(MetadataAction::FeelingsSave)),
        render::HintId::MoodDecrease => Some(Action::Metadata(MetadataAction::AdjustMood(-1))),
        render::HintId::MoodIncrease => Some(Action::Metadata(MetadataAction::AdjustMood(1))),
        render::HintId::MoodSave => Some(Action::Metadata(MetadataAction::MoodSave)),
        render::HintId::MoodClear => Some(Action::Metadata(MetadataAction::MoodClear)),
        render::HintId::LocationSwitchFocus => Some(Action::Location(LocationAction::SwitchFocus)),
        render::HintId::LocationResolve => Some(Action::Location(LocationAction::Resolve)),
        render::HintId::LocationGrabDevice => Some(Action::Location(LocationAction::GrabDevice)),
        render::HintId::LocationSelectRow => Some(Action::Location(LocationAction::SelectRow)),
        render::HintId::LocationSave => Some(Action::Location(LocationAction::Save)),
        render::HintId::LocationClear => Some(Action::Location(LocationAction::Clear)),
        render::HintId::OpenImageViewer if app.selected_entry_image_count() > 0 => {
            Some(Action::Images(ImageAction::OpenViewer(0)))
        }
        // The per-type metadata chips (and star) open their editor for the
        // selected entry, the same as the bare keys.
        render::HintId::EditTags if app.can_act_on_selected_entry() => Some(Action::Metadata(
            MetadataAction::BeginEdit(MetadataKind::Tags),
        )),
        render::HintId::EditPeople if app.can_act_on_selected_entry() => Some(Action::Metadata(
            MetadataAction::BeginEdit(MetadataKind::People),
        )),
        render::HintId::EditActivities if app.can_act_on_selected_entry() => Some(
            Action::Metadata(MetadataAction::BeginEdit(MetadataKind::Activities)),
        ),
        render::HintId::EditFeelings if app.can_act_on_selected_entry() => {
            Some(Action::Metadata(MetadataAction::BeginFeelings))
        }
        render::HintId::EditMood if app.can_act_on_selected_entry() => {
            Some(Action::Metadata(MetadataAction::BeginMood))
        }
        render::HintId::EditLocation if app.can_act_on_selected_entry() => {
            Some(Action::Location(LocationAction::BeginEdit))
        }
        render::HintId::ToggleStarred if app.can_act_on_selected_entry() => {
            Some(Action::Browser(BrowserAction::ToggleStarred))
        }
        render::HintId::ThemePickerApply => {
            Some(Action::Settings(SettingsAction::ThemePickerConfirm))
        }
        render::HintId::ThemePickerRevert => {
            Some(Action::Settings(SettingsAction::ThemePickerCancel))
        }
        render::HintId::ThemePickerChrome => {
            Some(Action::Settings(SettingsAction::ThemePickerCycleChrome))
        }
        render::HintId::ThemePickerMode => {
            Some(Action::Settings(SettingsAction::ThemePickerCycleMode))
        }
        render::HintId::ThemePickerScope => {
            Some(Action::Settings(SettingsAction::ThemePickerToggleScope))
        }
        render::HintId::Help => Some(Action::Overlay(OverlayAction::OpenHelp)),
        // Clicking the scope hint toggles it, only while the insights panel is focused.
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
        render::HintId::EditorSave => Some(Action::Editor(EditorAction::Save)),
        render::HintId::EditorDiscard => Some(Action::Editor(EditorAction::RequestDiscard)),
        render::HintId::EditorFullscreen => Some(Action::Editor(EditorAction::ToggleFullscreen)),
        render::HintId::EditorMetadata => Some(Action::Editor(EditorAction::OpenMetadataMenu)),
        render::HintId::EditorHelp => Some(Action::Editor(EditorAction::OpenHelp)),
        _ => None,
    }
}
