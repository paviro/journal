//! The tabbed insights panel. [`draw_journal_insights`] frames the panel with the
//! tab strip in its top border, resolves the memoized [`Analytics`] once, and
//! dispatches to one of the four tab renderers. The tab renderers (in the
//! sibling modules) are pure `(Rect, &Analytics) -> buffer` functions with no
//! `App` access, so they snapshot-test directly. Colour flows through
//! [`crate::tui::theme`]; the layout adapts to whatever `Rect` it is handed
//! (side column or expanded).

mod cadence;
mod correlate;
mod drivers;
mod feelings;
mod nav;
mod overview;
mod widgets;

pub(crate) use nav::{InsightsScope, InsightsTab, InsightsTimeframe};

use std::ops::Range;

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding},
};

use journal_storage::journal_display_name;

use crate::tui::app::{App, InsightsScrollGeometry};
use crate::tui::entry_rows::text_width;
use crate::tui::render::{render_centered_notice, render_scrollbar_if_needed};
use crate::tui::state::HoverTarget;
use crate::tui::surface::panel_content_inner;
use crate::tui::theme::theme;

pub(crate) fn draw_journal_insights(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    // Cleared each frame; the list tabs repopulate it so a stale scrollbar target
    // can't outlive the tab that drew it.
    app.insights_scroll = InsightsScrollGeometry::default();
    let focused = app.insights_panel_focused();
    let tab = app.nav.insights_tab;
    let hovered_tab = match app.hover {
        HoverTarget::InsightsTab(tab) => Some(tab),
        _ => None,
    };
    // The tabs live in the panel's top border. Scope only differentiates the
    // analytic tabs, so Overview leaves the bottom border unlabeled.
    let inner_width = area.width.saturating_sub(2);
    let flat = crate::tui::render::flat_chrome();
    let mut block = if flat {
        // Flat chrome: the tab strip sits on the top padding row instead of
        // the border; focus is carried by the tabs and the left stripe.
        Block::new()
            .style(Style::default().bg(theme().panel_bg()))
            .padding(Padding::uniform(1))
            .title(tabs_title_line(tab, focused, hovered_tab, inner_width))
    } else {
        let mut block = Block::default()
            .title(tabs_title_line(tab, focused, hovered_tab, inner_width))
            .borders(Borders::ALL);
        if focused {
            block = block
                .border_type(BorderType::Thick)
                .border_style(theme().focus_border());
        }
        block
    };
    if tab != InsightsTab::Overview {
        block = block.title_bottom(
            Line::from(format!(" {} ", app.nav.insights_scope.label())).right_aligned(),
        );
    }
    // The rolling window sits opposite the scope on tabs that respond to it.
    if tab.uses_timeframe() {
        block = block.title_bottom(
            Line::from(format!(" {} ", app.nav.insights_timeframe.label())).left_aligned(),
        );
    }
    let content = block.inner(area);
    frame.render_widget(block, area);
    crate::tui::render::panel_focus_stripe(frame, area, focused);
    if content.width == 0 || content.height == 0 {
        return;
    }
    // Match the other columns' one-cell horizontal padding, plus a one-line top
    // margin so content doesn't butt up against the border/tab strip. Tabs whose
    // first section is a heading already open with their own blank row, so they
    // skip this margin to avoid a doubled gap above the first title.
    let padded = panel_content_inner(content);
    let content = if tab.leads_with_heading() {
        padded
    } else {
        Rect {
            y: padded.y + 1,
            height: padded.height.saturating_sub(1),
            ..padded
        }
    };
    if content.height == 0 {
        return;
    }

    let Some(analytics) = app.cached_analytics() else {
        render_centered_notice(frame, content, "No journal selected");
        return;
    };

    match tab {
        InsightsTab::Overview => {
            let title = if app.nav.insights_scope == InsightsScope::All {
                "All journals".to_string()
            } else {
                app.selected_journal()
                    .map(|journal| journal_display_name(&journal.name).to_string())
                    .unwrap_or_default()
            };
            overview::draw(frame, content, &analytics, &title);
        }
        InsightsTab::Writing => cadence::draw(frame, content, &analytics),
        InsightsTab::Feelings => draw_scrollable(frame, area, app, |frame, scroll| {
            feelings::draw(frame, content, &analytics, scroll)
        }),
        InsightsTab::Drivers => {
            // Merge people/activities/tags for the selected window into one
            // lift/drain ranking; the Rc is dropped before the `&mut app` call.
            let rows = app
                .cached_windowed_correlations()
                .map(|correlations| drivers::rows(&correlations))
                .unwrap_or_default();
            // The trailing column is the feelings that ride with each driver.
            draw_scrollable(frame, area, app, |frame, scroll| {
                correlate::draw(frame, content, &rows, "No drivers yet", "Feelings", scroll)
            });
        }
    }
}

/// Render a scrollable list tab (Feelings, Drivers) via `draw`, threading the
/// panel's shared scroll offset through it, then draw the scrollbar and record its
/// geometry on the outer `panel` border so a mouse drag can map back to an offset.
fn draw_scrollable(
    frame: &mut Frame<'_>,
    panel: Rect,
    app: &mut App,
    draw: impl FnOnce(&mut Frame<'_>, &mut u16) -> correlate::InsightsListMetrics,
) {
    let mut scroll = app.nav.scroll.insights;
    let metrics = draw(frame, &mut scroll);
    app.nav.scroll.insights = scroll;
    render_scrollbar_if_needed(
        frame,
        panel,
        metrics.total,
        metrics.viewport as u16,
        scroll as usize,
    );
    app.insights_scroll = InsightsScrollGeometry {
        area: panel,
        total: metrics.total,
        viewport: metrics.viewport as u16,
    };
}

/// Which set of labels the tab strip is using at a given width.
#[derive(Clone, Copy)]
enum StripLevel {
    Full,
    Short,
    Initial,
}

/// Total strip width for a label function: a leading space, every label, and a
/// 3-cell ` · ` between each.
fn strip_width(label: impl Fn(InsightsTab) -> &'static str) -> usize {
    let labels: usize = InsightsTab::ALL
        .iter()
        .map(|tab| text_width(label(*tab)))
        .sum();
    1 + labels + 3 * (InsightsTab::ALL.len() - 1)
}

/// Pick the widest label set that fits `width`: full titles, then short titles,
/// then single-letter initials (which always fit).
fn strip_level(width: u16) -> StripLevel {
    let width = width as usize;
    if strip_width(InsightsTab::title) <= width {
        StripLevel::Full
    } else if strip_width(InsightsTab::short_title) <= width {
        StripLevel::Short
    } else {
        StripLevel::Initial
    }
}

/// The label for `tab` at the strip's current fit level.
fn tab_label(tab: InsightsTab, width: u16) -> &'static str {
    match strip_level(width) {
        StripLevel::Full => tab.title(),
        StripLevel::Short => tab.short_title(),
        StripLevel::Initial => tab.initial(),
    }
}

/// The column range each tab label occupies within a border title of `width`,
/// measured from the title's start (a leading space, then labels with a 3-cell
/// ` · ` between). The one source of truth shared by [`tabs_title_line`] and
/// [`insights_tab_at`] so drawing and hit-testing never drift.
fn tab_strip_segments(width: u16) -> Vec<(InsightsTab, Range<u16>)> {
    let mut segments = Vec::with_capacity(InsightsTab::ALL.len());
    let mut x: u16 = 1; // leading space
    for (index, tab) in InsightsTab::ALL.iter().enumerate() {
        if index > 0 {
            x += 3; // " · "
        }
        let w = text_width(tab_label(*tab, width)) as u16;
        segments.push((*tab, x..x + w));
        x += w;
    }
    segments
}

/// The tab bar as a border title: `Overview · Writing · Mood / Feelings · Drivers`
/// (short labels when they won't fit). The active tab is inverted while focused,
/// otherwise just bold; the rest stay dim.
fn tabs_title_line(
    active: InsightsTab,
    focused: bool,
    hovered: Option<InsightsTab>,
    width: u16,
) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (index, tab) in InsightsTab::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", theme().muted()));
        }
        let mut style = if *tab == active {
            theme().active_tab(focused)
        } else {
            theme().inactive_tab()
        };
        if hovered == Some(*tab) && *tab != active {
            style = tab_hover_style();
        }
        spans.push(Span::styled(tab_label(*tab, width).to_string(), style));
    }
    Line::from(spans)
}

fn tab_hover_style() -> Style {
    theme().text()
}

/// The tab whose border-title label covers `(column, row)`, or `None`. The strip
/// is the top border row; its title starts one cell past the corner.
pub(crate) fn insights_tab_at(area: Rect, column: u16, row: u16) -> Option<InsightsTab> {
    if row != area.y {
        return None;
    }
    let title_x = area.x + 1;
    let inner_width = area.width.saturating_sub(2);
    for (tab, range) in tab_strip_segments(inner_width) {
        if column >= title_x + range.start && column < title_x + range.end {
            return Some(tab);
        }
    }
    None
}

/// A signed value as `+1.2` / `-0.5`, one decimal.
pub(super) fn signed(value: f32) -> String {
    format!("{value:+.1}")
}
