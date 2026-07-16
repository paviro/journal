use std::time::{Duration, Instant};

use crate::tui::{app::AppModel, render};

use super::SEARCH_DEBOUNCE;

pub(super) fn poll_timeout(
    app: &AppModel,
    terminal_width: u16,
    pending_refresh_at: Option<Instant>,
    pending_theme_reload_at: Option<Instant>,
) -> Duration {
    let now = Instant::now();
    let mut timeout = app
        .toast_deadline()
        .map(|duration| duration.min(Duration::from_millis(200)))
        .unwrap_or(Duration::from_millis(200));

    if let Some(flash) = app.reader_anchor_flash.as_ref() {
        timeout = timeout.min(flash.until.saturating_duration_since(now));
    }
    if app.image.runtime.has_pending() {
        timeout = timeout.min(Duration::from_millis(30));
    }
    if app.geocode.has_pending() || app.address_backfill_active() {
        timeout = timeout.min(Duration::from_millis(50));
    }
    if app.environment.has_pending() || app.environment_backfill_active() {
        timeout = timeout.min(Duration::from_millis(100));
    }
    if !app.toasts.items().is_empty() {
        let columns = render::countdown_cols(terminal_width);
        if let Some(step) = app.toasts.next_countdown_step(columns) {
            timeout = timeout.min(step);
        }
    }
    if app.search.dirty {
        let remaining = app
            .search
            .last_edit
            .map(|edited| SEARCH_DEBOUNCE.saturating_sub(edited.elapsed()))
            .unwrap_or_default();
        timeout = timeout.min(remaining);
    }
    if let Some(at) = pending_refresh_at {
        timeout = timeout.min(at.saturating_duration_since(now));
    }
    if let Some(at) = pending_theme_reload_at {
        timeout = timeout.min(at.saturating_duration_since(now));
    }
    timeout
}
