use super::*;
use crate::{
    config::Config,
    tui::{
        app::{AppModel, Focus, ScrollbarDrag},
        features::{
            feelings::FeelingRow,
            insights::{InsightsTab, InsightsTimeframe},
            location::LocationPreset,
            metadata::EditMetadataFocus,
        },
        render, scroll,
        state::{HoverTarget, ListNav},
        test_support::{app_with_entries, app_with_entry, app_with_journals, new_app},
    },
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use notema_storage::JournalStore;
use ratatui::layout::Rect;
use std::fs;
use tempfile::tempdir;

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    }
}

fn down() -> MouseEventKind {
    MouseEventKind::Down(MouseButton::Left)
}

fn drag() -> MouseEventKind {
    MouseEventKind::Drag(MouseButton::Left)
}

fn up() -> MouseEventKind {
    MouseEventKind::Up(MouseButton::Left)
}

/// Render a frame into a fresh `ViewState`, returning it with the terminal so
/// callers can drive the production translation/dispatch paths against the
/// regions that render registered.
fn render_view(
    app: &mut AppModel,
    w: u16,
    h: u16,
) -> (
    ratatui::Terminal<ratatui::backend::TestBackend>,
    crate::tui::ui::ViewState,
) {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let mut view = crate::tui::ui::ViewState::default();
    let theme = app.appearance.theme.clone();
    terminal
        .draw(|frame| {
            let mut context = crate::tui::ui::RenderContext::new(&theme, &mut view);
            crate::tui::render::draw(frame, app, &mut context);
        })
        .unwrap();
    (terminal, view)
}

fn mouse_in_area(app: &mut AppModel, event: MouseEvent, w: u16, h: u16) {
    let (mut terminal, view) = render_view(app, w, h);
    if let Some(action) = mouse::mouse_to_action(app, event, Rect::new(0, 0, w, h), &view, false) {
        dispatch_action(&mut terminal, app, action).unwrap();
    }
}

/// The first cell whose topmost registered interaction region satisfies
/// `predicate`, scanning the frame row-major.
fn find_interaction(
    view: &crate::tui::ui::ViewState,
    w: u16,
    h: u16,
    predicate: impl Fn(&crate::tui::ui::InteractionKind) -> bool,
) -> Option<(u16, u16)> {
    (0..h)
        .flat_map(|row| (0..w).map(move |col| (col, row)))
        .find(|(col, row)| view.interactions.hit(*col, *row).is_some_and(&predicate))
}

/// Render a frame, then drive the production hover path at `(col, row)`.
/// Returns whether the hover target changed — the run loop's repaint signal.
fn apply_hover(app: &mut AppModel, col: u16, row: u16, area: Rect) -> bool {
    let (mut terminal, view) = render_view(app, area.width, area.height);
    mouse::update_hover(&mut terminal, app, col, row, area, &view).unwrap()
}

fn set_tag_dialog_items(app: &mut AppModel, count: usize) {
    let state = app.edit_metadata_state_mut().unwrap();
    state.all_values = (0..count)
        .map(|index| (format!("tag-{index:02}"), index + 1))
        .collect();
    state.filtered = (0..count).collect();
    state.normalize_list_state();
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

mod keyboard_cases;
mod mouse_cases;
mod overlay_cases;
mod paste_cases;
