//! The tabbed insights panel. [`draw_journal_insights`] frames the panel with the
//! tab strip in its top border, resolves the memoized [`Analytics`] once, and
//! dispatches to one of the four tab renderers. The tab renderers (in the
//! sibling modules) are pure `(Rect, &Analytics) -> buffer` functions with no
//! `AppModel` access, so they snapshot-test directly. Colour flows through
//! [`crate::tui::theme`]; the layout adapts to whatever `Rect` it is handed
//! (side column or expanded).

mod cadence;
mod correlate;
mod drivers;
mod feelings;
mod overview;
mod widgets;

use std::ops::Range;

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Padding},
};

use notema_storage::journal_display_name;

use crate::tui::app::{AppModel, InsightsScrollGeometry};
use crate::tui::entry_rows::text_width;
use crate::tui::features::insights::{InsightsScope, InsightsTab};
use crate::tui::render::{render_centered_notice, render_scrollbar_if_needed};
use crate::tui::state::HoverTarget;
use crate::tui::surface::surface_content_inner;
use crate::tui::theme::Theme;

pub(crate) fn draw_journal_insights(
    active_theme: &crate::tui::theme::Theme,
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut AppModel,
    geometry: &mut InsightsScrollGeometry,
) {
    let focused = app.insights_panel_focused();
    let tab = app.nav.insights_tab;
    let hovered_tab = match app.hover {
        HoverTarget::InsightsTab(tab) => Some(tab),
        _ => None,
    };
    // The tabs live in the panel's top border; a dimmed footnote in the bottom-left
    // (built below) names each analytic tab's scope and, where it applies, its rolling
    // window. Overview omits it — its top card already carries the scope.
    let inner_width = area.width.saturating_sub(2);
    let flat = crate::tui::render::flat_chrome(active_theme);
    let mut block = if flat {
        // Flat chrome: the tab strip sits on the top padding row instead of
        // the border; focus is carried by the tabs and the left stripe.
        Block::new()
            .style(Style::default().bg(active_theme.content_bg()))
            .padding(Padding::uniform(1))
            .title(tabs_title_line(
                active_theme,
                tab,
                focused,
                hovered_tab,
                inner_width,
            ))
    } else {
        let mut block = Block::default()
            .title(tabs_title_line(
                active_theme,
                tab,
                focused,
                hovered_tab,
                inner_width,
            ))
            .borders(Borders::ALL)
            .border_set(active_theme.glyphs().block_set(focused));
        if focused {
            block = block.border_style(active_theme.focus_border());
        } else {
            block = block.border_style(active_theme.inactive_border());
        }
        block
    };
    // A dimmed footnote in the bottom-left names the data the analytic tabs show: their
    // scope, plus the rolling window on tabs that respond to one. The separator carries
    // the tab-strip's separator colour, matching the top border. Overview skips it — its
    // top card already names the scope.
    if tab != InsightsTab::Overview {
        // The leading pad is unstyled so it doesn't lay a `DIM` cell under the
        // focus stripe's bottom row (ratatui accumulates cell modifiers, so a
        // dim space beneath the stripe would dim its final `┃`).
        let mut footnote = vec![
            Span::raw(" "),
            Span::styled(
                app.nav.insights_scope.label().to_string(),
                active_theme.muted(),
            ),
        ];
        if tab.uses_timeframe() {
            footnote.push(Span::styled(
                format!(" {} ", active_theme.glyphs().tab_separator),
                active_theme.tab_separator(),
            ));
            footnote.push(Span::styled(
                app.nav.insights_timeframe.label(),
                active_theme.muted(),
            ));
        }
        footnote.push(Span::styled(" ", active_theme.muted()));
        block = block.title_bottom(Line::from(footnote).left_aligned());
    }
    let content = block.inner(area);
    frame.render_widget(block, area);
    crate::tui::render::panel_focus_stripe(active_theme, frame, area, focused);
    if content.width == 0 || content.height == 0 {
        return;
    }
    // Match the other columns' one-cell horizontal padding, plus a one-line top
    // margin so content doesn't butt up against the border/tab strip.
    let padded = surface_content_inner(active_theme, content);
    let with_margin = Rect {
        y: padded.y + 1,
        height: padded.height.saturating_sub(1),
        ..padded
    };
    if with_margin.height == 0 {
        return;
    }

    // The empty-state notice centers within the margined area on every tab, so it
    // lands at the same height regardless of the tab's own layout.
    let Some(analytics) = app.cached_analytics() else {
        render_centered_notice(active_theme, frame, with_margin, "No journal selected");
        return;
    };

    // Tabs whose first section is a heading already open with their own blank row,
    // so they reclaim the top margin to avoid a doubled gap above the first title.
    let content = if tab.leads_with_heading() {
        padded
    } else {
        with_margin
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
            overview::draw(active_theme, frame, content, &analytics, &title);
        }
        InsightsTab::Writing => cadence::draw(active_theme, frame, content, &analytics),
        InsightsTab::Feelings => {
            draw_scrollable(active_theme, frame, area, app, geometry, |frame, scroll| {
                feelings::draw(active_theme, frame, content, &analytics, scroll)
            })
        }
        InsightsTab::Drivers => {
            // Merge people/activities/tags for the selected window into one
            // lift/drain ranking; the Rc is dropped before the `&mut app` call.
            let rows = app
                .cached_windowed_correlations()
                .map(|correlations| drivers::rows(&correlations))
                .unwrap_or_default();
            // The trailing column is the feelings that ride with each driver.
            draw_scrollable(active_theme, frame, area, app, geometry, |frame, scroll| {
                correlate::draw(
                    active_theme,
                    frame,
                    content,
                    &rows,
                    "No drivers yet",
                    "Feelings",
                    scroll,
                )
            });
        }
    }
}

/// Render a scrollable list tab (Feelings, Drivers) via `draw`, threading the
/// panel's shared scroll offset through it, then draw the scrollbar and record its
/// geometry on the outer `panel` border so a mouse drag can map back to an offset.
fn draw_scrollable(
    theme: &Theme,
    frame: &mut Frame<'_>,
    panel: Rect,
    app: &mut AppModel,
    geometry: &mut InsightsScrollGeometry,
    draw: impl FnOnce(&mut Frame<'_>, &mut u16) -> correlate::InsightsListMetrics,
) {
    let focused = app.insights_panel_focused();
    let mut scroll = app.nav.scroll.insights;
    let metrics = draw(frame, &mut scroll);
    render_scrollbar_if_needed(
        theme,
        frame,
        panel,
        metrics.total,
        metrics.viewport as u16,
        scroll as usize,
        focused,
    );
    *geometry = InsightsScrollGeometry {
        area: panel,
        total: metrics.total,
        viewport: metrics.viewport as u16,
        scroll,
    };
}

/// Which set of labels the tab strip is using at a given width.
#[derive(Clone, Copy)]
enum StripLevel {
    Full,
    Short,
    Initial,
}

/// Cells of leading space before the first tab label. Flat chrome indents by two
/// to line the strip up with the other columns' padded titles (which gained an
/// extra leading space); bordered chrome keeps the single space off the corner.
fn strip_leading(theme: &Theme) -> u16 {
    if crate::tui::render::flat_chrome(theme) {
        2
    } else {
        1
    }
}

/// Total strip width for a label function: the leading space(s), every label, and
/// a 3-cell ` · ` between each.
fn strip_width(theme: &Theme, label: impl Fn(InsightsTab) -> &'static str) -> usize {
    let labels: usize = InsightsTab::ALL
        .iter()
        .map(|tab| text_width(label(*tab)))
        .sum();
    strip_leading(theme) as usize + labels + 3 * (InsightsTab::ALL.len() - 1)
}

/// Pick the widest label set that fits `width`: full titles, then short titles,
/// then single-letter initials (which always fit).
fn strip_level(theme: &Theme, width: u16) -> StripLevel {
    let width = width as usize;
    if strip_width(theme, InsightsTab::title) <= width {
        StripLevel::Full
    } else if strip_width(theme, InsightsTab::short_title) <= width {
        StripLevel::Short
    } else {
        StripLevel::Initial
    }
}

/// The label for `tab` at the strip's current fit level.
fn tab_label(theme: &Theme, tab: InsightsTab, width: u16) -> &'static str {
    match strip_level(theme, width) {
        StripLevel::Full => tab.title(),
        StripLevel::Short => tab.short_title(),
        StripLevel::Initial => tab.initial(),
    }
}

/// The column range each tab label occupies within a border title of `width`,
/// measured from the title's start (a leading space, then labels with a 3-cell
/// ` · ` between). The one source of truth shared by [`tabs_title_line`] and
/// [`insights_tab_at`] so drawing and hit-testing never drift.
fn tab_strip_segments(theme: &Theme, width: u16) -> Vec<(InsightsTab, Range<u16>)> {
    let mut segments = Vec::with_capacity(InsightsTab::ALL.len());
    let mut x: u16 = strip_leading(theme); // leading space(s)
    for (index, tab) in InsightsTab::ALL.iter().enumerate() {
        if index > 0 {
            x += 3; // " · "
        }
        let w = text_width(tab_label(theme, *tab, width)) as u16;
        segments.push((*tab, x..x + w));
        x += w;
    }
    segments
}

/// The tab bar as a border title: `Overview · Writing · Mood / Feelings · Drivers`
/// (short labels when they won't fit). The active tab carries the focused-tab
/// style while focused (accent on flat chrome, inverted on bordered),
/// otherwise just bold; the rest stay dim.
fn tabs_title_line(
    theme: &Theme,
    active: InsightsTab,
    focused: bool,
    hovered: Option<InsightsTab>,
    width: u16,
) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(strip_leading(theme) as usize))];
    for (index, tab) in InsightsTab::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                format!(" {} ", theme.glyphs().tab_separator),
                theme.tab_separator(),
            ));
        }
        let mut style = if *tab == active {
            theme.active_tab(focused)
        } else {
            theme.inactive_tab()
        };
        if hovered == Some(*tab) && *tab != active {
            style = tab_hover_style(theme);
        }
        spans.push(Span::styled(
            tab_label(theme, *tab, width).to_string(),
            style,
        ));
    }
    Line::from(spans)
}

fn tab_hover_style(theme: &Theme) -> Style {
    theme.text()
}

/// The tab whose border-title label covers `(column, row)`, or `None`. The strip
/// is the top border row; its title starts one cell past the corner.
pub(crate) fn insights_tab_at(
    theme: &Theme,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<InsightsTab> {
    if row != area.y {
        return None;
    }
    let title_x = area.x + 1;
    let inner_width = area.width.saturating_sub(2);
    for (tab, range) in tab_strip_segments(theme, inner_width) {
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
