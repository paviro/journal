//! The UI's semantic style seam. Widgets ask the theme for *meaning*
//! (`heading`, `positive`, `accent`, …) and get back a ratatui [`Style`], never
//! a bare [`Color`]. This keeps colour out of the render code so a desktop
//! colour theme — or a black-and-white e-ink theme — can be swapped in from one
//! place.
//!
//! Monochrome contract: every semantic style also carries a non-colour modifier
//! (bold/dim/reversed) so it stays distinguishable with the palette stripped.
//! Meaning is conveyed by weight, glyph, sign, and bar length — never by hue
//! alone — so [`Palette::Monochrome`] changes appearance without hurting
//! legibility.

use ratatui::style::{Color, Modifier, Style};

/// Whether the theme emits colour. The layout reads the same either way; only
/// the `fg` colours drop out under [`Palette::Monochrome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Palette {
    Color,
    /// The seam for a future black-and-white e-ink theme. Constructed only in
    /// tests for now, but the whole design is built to stay legible under it.
    #[allow(dead_code)]
    Monochrome,
}

/// The active theme. A unit-ish handle for now — later it can carry a loaded
/// palette (accent hue, mood colours, …) without changing call sites.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Theme {
    palette: Palette,
}

/// The current global theme. Colour by default; a future config/e-ink toggle
/// swaps the palette here and the whole UI follows.
pub(crate) fn theme() -> Theme {
    Theme {
        palette: Palette::Color,
    }
}

impl Theme {
    #[cfg(test)]
    pub(crate) fn monochrome() -> Self {
        Self {
            palette: Palette::Monochrome,
        }
    }

    /// Apply `color` as a foreground only when the palette is coloured.
    fn tint(self, base: Style, color: Color) -> Style {
        match self.palette {
            Palette::Color => base.fg(color),
            Palette::Monochrome => base,
        }
    }

    /// Section titles and emphasised labels.
    pub(crate) fn heading(self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    /// Secondary text: captions, units, "+k more", empty hints.
    pub(crate) fn muted(self) -> Style {
        Style::default().add_modifier(Modifier::DIM)
    }

    /// The app's single accent — count bars, tags/feelings fills, series glyphs.
    /// Colour is decoration here: bars already encode magnitude by length.
    pub(crate) fn accent(self) -> Style {
        self.tint(Style::default(), Color::Cyan)
    }

    /// Alias of [`Self::accent`] for count/frequency bar fills.
    pub(crate) fn bar_fill(self) -> Style {
        self.accent()
    }

    /// A positive/above-zero value. Bold so it survives monochrome; sign and bar
    /// direction carry the meaning, the green is decoration.
    pub(crate) fn positive(self) -> Style {
        self.tint(Style::default().add_modifier(Modifier::BOLD), Color::Green)
    }

    /// A negative/below-zero value. Bold + red; see [`Self::positive`].
    pub(crate) fn negative(self) -> Style {
        self.tint(Style::default().add_modifier(Modifier::BOLD), Color::Red)
    }

    /// A neutral/at-zero value.
    pub(crate) fn neutral(self) -> Style {
        Style::default()
    }

    /// Style a signed value (mood, mood delta, trend) by its sign. The single
    /// place +/- becomes a style, so the whole panel stays consistent.
    pub(crate) fn signed(self, value: f32) -> Style {
        if value > 0.0 {
            self.positive()
        } else if value < 0.0 {
            self.negative()
        } else {
            self.neutral()
        }
    }

    /// The active tab in the tab strip: inverted while the panel is focused so
    /// the selection reads even without colour, otherwise just bold.
    pub(crate) fn active_tab(self, focused: bool) -> Style {
        if focused {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        }
    }

    /// A non-active tab.
    pub(crate) fn inactive_tab(self) -> Style {
        Style::default().add_modifier(Modifier::DIM)
    }

    /// The border of the focused panel, paired with its thick border type so focus
    /// reads without colour.
    pub(crate) fn focus_border(self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    /// The inter-row grid lines of a table, drawn fainter than the outer borders
    /// and header rule so the rows separate without the grid competing with the
    /// data. Colour uses a dim grey; monochrome falls back to plain [`Self::muted`]
    /// (DIM is already its faintest ink).
    pub(crate) fn faint_rule(self) -> Style {
        match self.palette {
            Palette::Color => Style::default().fg(Color::Indexed(240)),
            Palette::Monochrome => self.muted(),
        }
    }

    /// A recessed box outline — a touch brighter than [`Self::faint_rule`] so card
    /// and panel borders read as present-but-quiet rather than disappearing. Grey
    /// on colour, DIM on monochrome.
    pub(crate) fn card_border(self) -> Style {
        match self.palette {
            Palette::Color => Style::default().fg(Color::Indexed(244)),
            Palette::Monochrome => self.muted(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_distinguishes_positive_from_negative() {
        let theme = theme();
        assert_ne!(theme.signed(1.0), theme.signed(-1.0));
    }

    #[test]
    fn positive_and_negative_survive_monochrome() {
        // With colour stripped, the two still differ from muted text and from
        // each other's absence of colour only by weight — both must stay bold so
        // a signed value never renders as plain body text on e-ink.
        let mono = Theme::monochrome();
        assert!(mono.positive().add_modifier.contains(Modifier::BOLD));
        assert!(mono.negative().add_modifier.contains(Modifier::BOLD));
        assert_eq!(mono.positive().fg, None);
        assert_eq!(mono.negative().fg, None);
    }
}
