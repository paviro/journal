//! Navigation state for the tabbed insights panel: which tab is showing, the
//! rolling timeframe the mood-driver views window to, and whether the analytic
//! tabs aggregate the selected journal or every journal. The aggregation itself
//! lives in the `journal-analytics` crate; this file is only the UI-side enums
//! that `insights/` renders and the event layer drives.

use chrono::{Duration, NaiveDate};

/// Which insight the panel is showing. `Overview` is an at-a-glance dashboard;
/// the rest each sharpen the analytics toward "what makes me feel good/bad".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum InsightsTab {
    #[default]
    Overview,
    /// Writing habits: streaks, when you write, and word volume.
    Writing,
    /// Mood balance, the signed mood breakdowns, and the feeling frequency table.
    Feelings,
    /// People, activities, and tags merged into one lift/drain ranking.
    Drivers,
}

impl InsightsTab {
    pub(crate) const ALL: [InsightsTab; 4] =
        [Self::Overview, Self::Writing, Self::Feelings, Self::Drivers];

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    /// Whether this tab renders a scrollable list (Drivers' ranking, Feelings'
    /// frequency table) rather than a fixed dashboard — the tabs that record
    /// scrollbar geometry and respond to arrow/page keys.
    pub(crate) fn is_list(self) -> bool {
        matches!(self, Self::Drivers | Self::Feelings)
    }

    /// Whether this tab's first section is a [`heading`](super::widgets)-led block,
    /// which already opens with its own blank row. Such tabs skip the panel's top
    /// margin so the first title sits one row below the border, not two.
    pub(crate) fn leads_with_heading(self) -> bool {
        matches!(self, Self::Feelings)
    }

    /// Whether this tab windows to the selected [`InsightsTimeframe`]. Only Drivers:
    /// Feelings' Balance shows all windows at once, so its `w` toggle would be a
    /// no-op.
    pub(crate) fn uses_timeframe(self) -> bool {
        matches!(self, Self::Drivers)
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Writing => "Writing",
            Self::Feelings => "Mood / Feelings",
            Self::Drivers => "Drivers",
        }
    }

    /// A short label used when the full titles won't fit the tab strip.
    pub(crate) fn short_title(self) -> &'static str {
        match self {
            Self::Overview => "Over",
            Self::Writing => "Writ",
            Self::Feelings => "Mood",
            Self::Drivers => "Driv",
        }
    }

    /// A single-letter label — the last-resort tab strip rung. Each is unique, so
    /// every tab stays visible and clickable even on the narrowest panel.
    pub(crate) fn initial(self) -> &'static str {
        match self {
            Self::Overview => "O",
            Self::Writing => "W",
            Self::Feelings => "M",
            Self::Drivers => "D",
        }
    }

    pub(crate) fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub(crate) fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// The rolling time window the mood-driver views (Drivers, Feelings' Balance)
/// aggregate over. Rolling rather than calendar so every window is a full span
/// regardless of where "today" sits in the month, and no locale/week-start
/// config is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum InsightsTimeframe {
    #[default]
    Overall,
    Year,
    Month,
    Week,
}

impl InsightsTimeframe {
    pub(crate) const ALL: [InsightsTimeframe; 4] =
        [Self::Overall, Self::Year, Self::Month, Self::Week];

    fn index(self) -> usize {
        Self::ALL.iter().position(|tf| *tf == self).unwrap_or(0)
    }

    /// The window cycles forward-only from a single `w` key, so there is no
    /// `prev`; the set is small enough that forward stepping reaches any window.
    pub(crate) fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Overall => "All time",
            Self::Year => "This year",
            Self::Month => "This month",
            Self::Week => "This week",
        }
    }

    /// The inclusive `(start, today)` date range this timeframe covers, or `None`
    /// for `Overall` (no filtering). Rolling windows: 365 / 30 / 7 days ending on
    /// (and including) `today`.
    pub(crate) fn window(self, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        let days = match self {
            Self::Overall => return None,
            Self::Year => 365,
            Self::Month => 30,
            Self::Week => 7,
        };
        Some((today - Duration::days(days - 1), today))
    }
}

/// Whether the analytic tabs aggregate only the selected journal or every
/// journal. `Overview` ignores this (it's always the selected journal's card).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum InsightsScope {
    #[default]
    Journal,
    All,
}

impl InsightsScope {
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Journal => Self::All,
            Self::All => Self::Journal,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Journal => "This journal",
            Self::All => "All journals",
        }
    }
}
