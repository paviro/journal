//! The UI's semantic style seam. Widgets ask the theme for *meaning*
//! (`heading`, `positive`, `primary`, …) and get back a ratatui [`Style`], never
//! a bare [`Color`]. Themes are TOML files in `<config-dir>/themes/`; the
//! bundled ones are materialized there on first launch and stay user-editable.
//!
//! Every color in a theme file may be a single value or a `{ dark, light }`
//! pair; resolution against the terminal's detected [`Mode`] happens once at
//! load, so [`theme()`] reads never branch.
//!
//! Monochrome contract: the modifiers that carry meaning (bold on signed
//! values, dim on secondary ink, inversion on selection fallbacks) are applied
//! in code, not read from theme data, so no theme file can make a positive
//! value render as plain body text on e-ink.

mod schema;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use ratatui::style::{Color, Modifier, Style};
use schema::{ThemeFile, parse};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(not(test))]
use std::sync::RwLock;

/// The bundled themes, embedded so the binary can materialize and fall back to
/// them without touching the network or the repo.
const BUNDLED: [(&str, &str); 15] = [
    ("journal", include_str!("../themes/journal.toml")),
    ("classic", include_str!("../themes/classic.toml")),
    ("e-ink", include_str!("../themes/e-ink.toml")),
    ("blossom", include_str!("../themes/blossom.toml")),
    ("fjord", include_str!("../themes/fjord.toml")),
    ("grove", include_str!("../themes/grove.toml")),
    ("tokyonight", include_str!("../themes/tokyonight.toml")),
    ("catppuccin", include_str!("../themes/catppuccin.toml")),
    ("matcha", include_str!("../themes/matcha.toml")),
    ("rose-pine", include_str!("../themes/rose-pine.toml")),
    ("dungeon", include_str!("../themes/dungeon.toml")),
    ("synthwave", include_str!("../themes/synthwave.toml")),
    ("crt", include_str!("../themes/crt.toml")),
    ("cyberpunk", include_str!("../themes/cyberpunk.toml")),
    ("vaporwave", include_str!("../themes/vaporwave.toml")),
];

/// The theme `load` falls back to when the configured one is missing or broken.
pub(crate) const DEFAULT_THEME: &str = "blossom";

/// Which variant of a `{ dark, light }` color a load resolves to. Detected from
/// the terminal background once at startup and cached for the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Dark,
    Light,
}

/// A chart fill: which glyph is repeated and how it is styled. E-ink themes
/// vary the glyph per series so data stays readable without hue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Fill {
    pub(crate) glyph: char,
    pub(crate) style: Style,
}

/// How a theme wants its chrome drawn: `Flat` separates surfaces by background
/// layers (opencode-style), `Bordered` keeps the classic drawn borders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ChromeStyle {
    Flat,
    Bordered,
}

/// The line character set a theme draws boxes with: panel and dialog borders,
/// the hand-drawn entry/journal cards, and table grids.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BorderGlyphs {
    #[default]
    Plain,
    Rounded,
    Double,
    Thick,
    Ascii,
    /// A theme-authored set (`[borders.glyphs]`), assembled by the schema.
    /// Never spellable as `style = "custom"` — it only exists resolved.
    #[serde(skip)]
    Custom(&'static CustomBorderSet),
}

/// The ratatui sets a `[borders.glyphs]` table resolves to, interned by the
/// schema so [`BorderGlyphs`] stays pointer-sized and `Copy`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CustomBorderSet {
    pub(super) border: ratatui::symbols::border::Set<'static>,
    pub(super) line: ratatui::symbols::line::Set<'static>,
}

/// The `+-|` sets for [`BorderGlyphs::Ascii`], for terminals or looks that
/// want no box-drawing characters at all.
const ASCII_BORDER_SET: ratatui::symbols::border::Set<'static> = ratatui::symbols::border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

const ASCII_LINE_SET: ratatui::symbols::line::Set<'static> = ratatui::symbols::line::Set {
    vertical: "|",
    horizontal: "-",
    top_right: "+",
    top_left: "+",
    bottom_right: "+",
    bottom_left: "+",
    vertical_left: "+",
    vertical_right: "+",
    horizontal_down: "+",
    horizontal_up: "+",
    cross: "+",
};

impl BorderGlyphs {
    /// The set ratatui `Block` borders draw with.
    pub(crate) fn border_set(self) -> ratatui::symbols::border::Set<'static> {
        use ratatui::symbols::border;
        match self {
            BorderGlyphs::Plain => border::PLAIN,
            BorderGlyphs::Rounded => border::ROUNDED,
            BorderGlyphs::Double => border::DOUBLE,
            BorderGlyphs::Thick => border::THICK,
            BorderGlyphs::Ascii => ASCII_BORDER_SET,
            BorderGlyphs::Custom(set) => set.border,
        }
    }

    /// The full line set (corners, junctions, cross) for hand-drawn boxes and
    /// table grids.
    pub(crate) fn line_set(self) -> ratatui::symbols::line::Set<'static> {
        use ratatui::symbols::line;
        match self {
            BorderGlyphs::Plain => line::NORMAL,
            BorderGlyphs::Rounded => line::ROUNDED,
            BorderGlyphs::Double => line::DOUBLE,
            BorderGlyphs::Thick => line::THICK,
            BorderGlyphs::Ascii => ASCII_LINE_SET,
            BorderGlyphs::Custom(set) => set.line,
        }
    }

    /// The `Block` border set for a panel, thickened when focused — thickness
    /// is how focus survives monochrome. Ascii and custom sets have no thick
    /// variant; there focus is carried by the bold border style alone.
    pub(crate) fn block_set(self, focused: bool) -> ratatui::symbols::border::Set<'static> {
        let promotes = matches!(
            self,
            BorderGlyphs::Plain
                | BorderGlyphs::Rounded
                | BorderGlyphs::Double
                | BorderGlyphs::Thick
        );
        if focused && promotes {
            BorderGlyphs::Thick.border_set()
        } else {
            self.border_set()
        }
    }
}

/// Resolved syntax-highlight colors for fenced code blocks. `Reset` means the
/// category renders in the plain code style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Syntax {
    pub(crate) comment: Color,
    pub(crate) keyword: Color,
    pub(crate) string: Color,
    pub(crate) string_escape: Color,
    pub(crate) number: Color,
    pub(crate) constant: Color,
    pub(crate) function: Color,
    pub(crate) r#type: Color,
    pub(crate) variable: Color,
    pub(crate) property: Color,
    pub(crate) operator: Color,
    pub(crate) punctuation: Color,
    pub(crate) attribute: Color,
    pub(crate) tag: Color,
    pub(crate) label: Color,
    pub(crate) error: Color,
}

impl Syntax {
    /// Whether the theme colors any category at all. Plain themes skip the
    /// highlighter entirely, keeping their classic un-highlighted code blocks.
    pub(crate) fn any_color(self) -> bool {
        // Keep this list in sync with the struct fields.
        [
            self.comment,
            self.keyword,
            self.string,
            self.string_escape,
            self.number,
            self.constant,
            self.function,
            self.r#type,
            self.variable,
            self.property,
            self.operator,
            self.punctuation,
            self.attribute,
            self.tag,
            self.label,
            self.error,
        ]
        .into_iter()
        .any(|color| color != Color::Reset)
    }
}

/// The theme's identity glyphs — every meaning-free character the UI repeats.
/// Meaning-carrying glyph *variance* (heavy vs light at a zero mood, distinct
/// chart-series glyphs) stays in code and [`Fill`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Glyphs {
    /// The marker before a selected list row. `None` follows the chrome
    /// (`●` flat, `>` bordered); resolved by [`Theme::selection_marker`].
    pub(crate) selection_marker: Option<char>,
    /// The stripe down a focused panel's left edge (flat chrome).
    pub(crate) focus_stripe: char,
    /// The accent edges of a toast card (flat chrome).
    pub(crate) toast_edge: char,
    /// The separator between tab labels; always rendered with a space each
    /// side so the strip's width math stays fixed.
    pub(crate) tab_separator: char,
    /// The rule of section dividers (month headers, "Archived").
    pub(crate) divider: char,
    /// The zero-baseline decoration of column charts (`charts.baseline.glyph`).
    pub(crate) chart_baseline: char,
    /// The groove marking an empty delta bar (`charts.groove`).
    pub(crate) chart_groove: char,
    /// The center marker of delta/mood bars (`charts.bar_center`). The heavy
    /// variant shown at an exact zero stays code-side (weight carries meaning).
    pub(crate) bar_center: char,
    /// The stroke of the mood bar's fill (`charts.mood_stroke`).
    pub(crate) mood_fill: char,
    /// The scrollbar's draggable handle (`glyphs.scrollbar_thumb`).
    pub(crate) scrollbar_thumb: char,
    /// The scrollbar's track behind the handle (`glyphs.scrollbar_track`).
    pub(crate) scrollbar_track: char,
    /// The arrow capping the scrollbar's top (`glyphs.scrollbar_up`).
    pub(crate) scrollbar_up: char,
    /// The arrow capping the scrollbar's bottom (`glyphs.scrollbar_down`).
    pub(crate) scrollbar_down: char,
    /// The box-drawing set for borders, cards, and table grids (`borders.style`
    /// or `[borders.glyphs]`).
    pub(crate) borders: BorderGlyphs,
    /// What a focused panel's border is drawn with (`borders.focused_style` /
    /// `[borders.focused_glyphs]`). `None` keeps the classic thick promotion.
    pub(crate) focused_borders: Option<BorderGlyphs>,
}

impl Glyphs {
    /// The border set for a panel: the theme's focus override when focused,
    /// otherwise the base set with its thick promotion.
    pub(crate) fn block_set(self, focused: bool) -> ratatui::symbols::border::Set<'static> {
        match (focused, self.focused_borders) {
            (true, Some(borders)) => borders.border_set(),
            _ => self.borders.block_set(focused),
        }
    }
}

/// A fully resolved theme: plain styles and colors, no variants left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Theme {
    bg: Color,
    panel: Color,
    dialog: Color,
    element: Color,
    text: Style,
    muted: Style,
    heading: Style,
    placeholder: Style,
    primary: Style,
    secondary: Style,
    border: Style,
    border_subtle: Style,
    border_active: Style,
    border_inactive: Style,
    divider_style: Style,
    card_border: Style,
    tab_separator_style: Style,
    success: Style,
    warning: Style,
    error: Style,
    info: Style,
    selection: Style,
    hover: Style,
    button: Style,
    button_hover: Style,
    key_hint: Style,
    cursor: Style,
    cursor_line: Style,
    scrollbar_thumb: Style,
    scrollbar_track: Style,
    scrollbar_arrow: Style,
    chart_positive: Fill,
    chart_neutral: Fill,
    chart_negative: Fill,
    bar: Fill,
    track: Fill,
    chart_baseline: Style,
    chart_label: Style,
    md_heading: Style,
    md_heading3: Style,
    md_link: Style,
    md_code: Style,
    md_blockquote: Style,
    syntax: Syntax,
    glyphs: Glyphs,
    chrome: ChromeStyle,
    scrim: f32,
}

/// One session-scoped state slot: a process-global `RwLock` in production, a
/// per-thread `Cell` under test so parallel tests can't restyle each other
/// (each `#[test]` runs on its own thread, so a set value never leaks into
/// another test). Thread-locals can't be generic, so test mode injects a
/// `&'static LocalKey` declared alongside the static.
struct SessionCell<T: Copy + 'static> {
    #[cfg(not(test))]
    slot: RwLock<Option<T>>,
    #[cfg(test)]
    slot: &'static std::thread::LocalKey<std::cell::Cell<Option<T>>>,
}

impl<T: Copy + 'static> SessionCell<T> {
    #[cfg(not(test))]
    const fn new() -> Self {
        Self {
            slot: RwLock::new(None),
        }
    }

    #[cfg(test)]
    const fn new(slot: &'static std::thread::LocalKey<std::cell::Cell<Option<T>>>) -> Self {
        Self { slot }
    }

    fn get(&self) -> Option<T> {
        #[cfg(test)]
        return self.slot.with(std::cell::Cell::get);
        #[cfg(not(test))]
        *self.slot.read().expect("session cell lock")
    }

    fn set(&self, value: Option<T>) {
        #[cfg(test)]
        self.slot.with(|cell| cell.set(value));
        #[cfg(not(test))]
        {
            *self.slot.write().expect("session cell lock") = value;
        }
    }
}

#[cfg(test)]
thread_local! {
    static THEME_SLOT: std::cell::Cell<Option<Theme>> = const { std::cell::Cell::new(None) };
    static CHROME_SLOT: std::cell::Cell<Option<ChromeStyle>> =
        const { std::cell::Cell::new(None) };
    static COLOR_MODE_SLOT: std::cell::Cell<Option<crate::config::ColorMode>> =
        const { std::cell::Cell::new(None) };
}

/// The installed theme. `None` until [`install`] runs; readers fall back to
/// [`Theme::terminal_default`], which is what every test exercises.
#[cfg(not(test))]
static THEME: SessionCell<Theme> = SessionCell::new();
#[cfg(test)]
static THEME: SessionCell<Theme> = SessionCell::new(&THEME_SLOT);

/// The user's chrome override (`[ui] chrome = "flat"|"bordered"`), applied on
/// top of whatever the active theme declares as its `chrome.default_style`. `None`
/// (= `default`) follows the theme. Runtime-writable so the theme picker can
/// cycle it with live preview.
#[cfg(not(test))]
static CHROME_OVERRIDE: SessionCell<ChromeStyle> = SessionCell::new();
#[cfg(test)]
static CHROME_OVERRIDE: SessionCell<ChromeStyle> = SessionCell::new(&CHROME_SLOT);

/// The forced chrome style, or `None` when following the theme (`default`).
pub(crate) fn chrome_override() -> Option<ChromeStyle> {
    CHROME_OVERRIDE.get()
}

/// Force a chrome style on every theme (`None` = follow the theme). The next
/// frame repaints with it — `theme()` applies it on read.
pub(crate) fn set_chrome_override(style: Option<ChromeStyle>) {
    CHROME_OVERRIDE.set(style);
}

/// The current theme, with the chrome override applied. Cheap to call
/// everywhere: `Theme` is `Copy`.
pub(crate) fn theme() -> Theme {
    let mut theme = THEME.get().unwrap_or_else(Theme::terminal_default);
    if let Some(style) = chrome_override() {
        theme.chrome = style;
    }
    theme
}

/// Swap the active theme; the next frame repaints with it. Used at startup,
/// by live reload, and by the theme picker's preview. Under test it swaps a
/// per-thread slot instead, so parallel tests can't restyle each other.
pub(crate) fn install(theme: Theme) {
    THEME.set(Some(theme));
}

/// The session's color-mode setting (`[ui] color_mode`). Runtime-writable so
/// the theme picker can cycle auto/dark/light with live preview; `Auto` until
/// [`init_from_config`] runs.
#[cfg(not(test))]
static COLOR_MODE: SessionCell<crate::config::ColorMode> = SessionCell::new();
#[cfg(test)]
static COLOR_MODE: SessionCell<crate::config::ColorMode> = SessionCell::new(&COLOR_MODE_SLOT);

/// The terminal background detected at startup, which is what `auto` resolves
/// to. Detection must happen before raw mode / the alternate screen (it talks
/// OSC to the normal screen), so mid-session mode switches reuse this answer.
static DETECTED: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();

/// The session's color-mode setting.
pub(crate) fn color_mode() -> crate::config::ColorMode {
    COLOR_MODE.get().unwrap_or_default()
}

/// Switch the session's color mode. Callers re-resolve and re-[`install`] the
/// active theme themselves — a resolved [`Theme`] has no variants left to swap.
pub(crate) fn set_color_mode(color_mode: crate::config::ColorMode) {
    COLOR_MODE.set(Some(color_mode));
}

/// The dark/light mode theme files resolve against: the explicit setting, or
/// the detected terminal background for `auto` (dark when unknown).
pub(crate) fn mode() -> Mode {
    use crate::config::ColorMode;
    match color_mode() {
        ColorMode::Dark => Mode::Dark,
        ColorMode::Light => Mode::Light,
        ColorMode::Auto => DETECTED.get().copied().unwrap_or(Mode::Dark),
    }
}

/// Detect the terminal background, then load and install the configured theme.
/// Must run before the terminal enters raw mode / the alternate screen: the
/// detection talks OSC to the normal screen. Detection always runs (not just
/// on `auto`) so the picker can switch to `auto` mid-session.
pub(crate) fn init_from_config(config_path: &Path, ui: &crate::config::UiSection) {
    let _ = DETECTED.set(detect_terminal_background());
    set_color_mode(ui.color_mode);
    set_chrome_override(ui.chrome.forced_style());
    install(load(config_path, &ui.theme, mode()));
}

/// Ask the terminal for its background (OSC 10/11, with the library's own
/// support heuristic and timeout); unknown counts as dark.
fn detect_terminal_background() -> Mode {
    match terminal_colorsaurus::theme_mode(terminal_colorsaurus::QueryOptions::default()) {
        Ok(terminal_colorsaurus::ThemeMode::Light) => Mode::Light,
        Ok(terminal_colorsaurus::ThemeMode::Dark) | Err(_) => Mode::Dark,
    }
}

/// Pin the theme seen by `theme()` on this test thread.
#[cfg(test)]
pub(crate) fn set_test_theme(theme: Theme) {
    install(theme);
}

/// The resolved bundled default (flat-chrome) theme, for render tests that
/// exercise the bg-layered chrome path via [`set_test_theme`].
#[cfg(test)]
pub(crate) fn test_flat_theme() -> Theme {
    builtin(DEFAULT_THEME, Mode::Dark).expect("bundled default theme resolves")
}

/// The resolved bundled e-ink theme, for tests asserting the monochrome
/// glyph-differentiation contract end to end.
#[cfg(test)]
pub(crate) fn test_eink_theme() -> Theme {
    builtin("e-ink", Mode::Dark).expect("bundled e-ink theme resolves")
}

/// Resolve a theme snippet (dark mode) for tests that pin specific tokens.
#[cfg(test)]
pub(crate) fn test_theme_from_toml(text: &str) -> Theme {
    parse(text, Mode::Dark).expect("test theme snippet resolves")
}

impl Theme {
    /// The look the app has always had on a bare terminal: default colors,
    /// bordered chrome, meaning carried by modifiers. This is byte-for-byte
    /// the resolved `classic.toml` (a test pins that) and the fallback when no
    /// theme has been installed or the configured one fails to parse.
    pub(crate) fn terminal_default() -> Self {
        ThemeFile::default()
            .resolve(Mode::Dark)
            .expect("default theme resolves")
    }

    // --- surfaces ---

    /// The application background, painted under every frame.
    pub(crate) fn bg(self) -> Color {
        self.bg
    }

    /// Elevated surfaces: panels, dialogs, notices, toasts.
    pub(crate) fn panel_bg(self) -> Color {
        self.panel
    }

    /// Dialog surfaces, defaulting to the panel surface unless a theme splits them.
    pub(crate) fn dialog_bg(self) -> Color {
        self.dialog
    }

    /// Interactive surfaces sitting on a panel: inputs, active controls.
    pub(crate) fn element_bg(self) -> Color {
        self.element
    }

    // --- text ---

    /// Primary body text.
    pub(crate) fn text(self) -> Style {
        self.text
    }

    /// Section titles and emphasised labels. Bold in every theme — weight is
    /// how headings survive monochrome — but the ink is the theme's to pick.
    pub(crate) fn heading(self) -> Style {
        self.heading.add_modifier(Modifier::BOLD)
    }

    /// Secondary text: captions, units, "+k more", empty hints.
    pub(crate) fn muted(self) -> Style {
        self.muted.add_modifier(Modifier::DIM)
    }

    /// Placeholder text in empty inputs. Dim in every theme so a prompt never
    /// reads as entered text.
    pub(crate) fn placeholder(self) -> Style {
        self.placeholder.add_modifier(Modifier::DIM)
    }

    // --- accents ---

    /// The primary accent as a style: focused titles, current-item markers.
    pub(crate) fn primary(self) -> Style {
        self.primary
    }

    /// The second accent hue: the active tab, and anywhere a theme wants a hero
    /// color distinct from `primary`. Defaults to `primary` when a theme sets no
    /// `accents.secondary`, so single-accent themes are unaffected. A third hue,
    /// `accents.tertiary`, has no dedicated render site but is seeded as a
    /// palette name (`fg = "tertiary"`) alongside `primary`/`secondary`.
    pub(crate) fn secondary(self) -> Style {
        self.secondary
    }

    // --- signed / status values ---

    /// A positive/above-zero value. Bold in every theme so it survives
    /// monochrome; sign and bar direction carry the meaning.
    pub(crate) fn positive(self) -> Style {
        self.chart_positive.style
    }

    /// A negative/below-zero value. Bold; see [`Self::positive`].
    pub(crate) fn negative(self) -> Style {
        self.chart_negative.style
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

    /// A success/confirmation state (toasts, status glyphs).
    pub(crate) fn success(self) -> Style {
        self.success
    }

    /// A warning state.
    pub(crate) fn warning(self) -> Style {
        self.warning
    }

    /// An error state.
    pub(crate) fn error(self) -> Style {
        self.error
    }

    /// An informational state.
    pub(crate) fn info(self) -> Style {
        self.info
    }

    // --- interaction ---

    /// The selected row/item. Flat themes fill with the primary hue and an
    /// explicit contrast foreground; the fallback stays REVERSED so selection
    /// reads without color.
    pub(crate) fn selection(self) -> Style {
        self.selection
    }

    /// The row/chip under the mouse cursor. Defaults to the element surface,
    /// which resolves to the terminal default on classic/bordered themes (no
    /// visible hover) and to a subtle lift on flat themes. Layers under the
    /// row's existing ink, so no contrast foreground is required.
    pub(crate) fn hover(self) -> Style {
        self.hover
    }

    /// A primary action button chip.
    pub(crate) fn button(self) -> Style {
        self.button
    }

    /// The style patched onto a button chip under the mouse. Defaults to an
    /// underline; a theme can restyle it via `interaction.button_hover`.
    pub(crate) fn button_hover(self) -> Style {
        self.button_hover
    }

    /// A keybinding chip/hint in the footer and dialogs.
    pub(crate) fn key_hint(self) -> Style {
        self.key_hint
    }

    /// The editor/input cursor while *not* selecting. The REVERSED block shown
    /// during a selection stays code-enforced so a selection always reads.
    pub(crate) fn cursor(self) -> Style {
        self.cursor
    }

    /// The line under the cursor in the multi-line editor. Defaults to no
    /// highlight; themes may add a subtle background tint.
    pub(crate) fn cursor_line(self) -> Style {
        self.cursor_line
    }

    /// The active tab in the tab strip while the panel is focused: the secondary
    /// accent + bold on flat chrome (a theme can split it from the primary hue
    /// its titles use; it falls back to primary), selection-styled on bordered
    /// chrome so it reads even without colour. Unfocused it's just bold either
    /// way, so it still stands apart from the muted inactive tabs.
    pub(crate) fn active_tab(self, focused: bool) -> Style {
        if !focused {
            return Style::default().add_modifier(Modifier::BOLD);
        }
        if self.chrome == ChromeStyle::Flat {
            self.secondary().add_modifier(Modifier::BOLD)
        } else {
            self.selection.add_modifier(Modifier::BOLD)
        }
    }

    /// A non-active tab.
    pub(crate) fn inactive_tab(self) -> Style {
        self.muted()
    }

    /// The ink of the separator glyph between tab labels. Defaults to the muted
    /// ink unless a theme sets `tabs.separator_style`.
    pub(crate) fn tab_separator_style(self) -> Style {
        self.tab_separator_style
    }

    // --- borders ---

    /// The border of the focused panel, paired with its thick border type so
    /// focus reads without colour.
    pub(crate) fn focus_border(self) -> Style {
        self.border_active.add_modifier(Modifier::BOLD)
    }

    /// The border of an unfocused panel; pairs with [`Self::focus_border`].
    pub(crate) fn inactive_border(self) -> Style {
        self.border_inactive
    }

    /// The frame of a dialog or full-screen modal: the active surface's hue
    /// without the focused panel's bold weight.
    pub(crate) fn dialog_border(self) -> Style {
        self.border_active
    }

    /// The inter-row grid lines of a table, drawn fainter than the outer
    /// borders and header rule so the rows separate without the grid competing
    /// with the data.
    pub(crate) fn faint_rule(self) -> Style {
        self.border_subtle
    }

    /// The rule of a section divider (month headers, the "Archived" break).
    /// Defaults to the muted ink; a theme can give it a hue via
    /// `borders.divider_style`.
    pub(crate) fn divider_style(self) -> Style {
        self.divider_style
    }

    /// A recessed box outline — a touch brighter than [`Self::faint_rule`] so
    /// card and panel borders read as present-but-quiet. Defaults to the normal
    /// border unless a theme sets `borders.card`.
    pub(crate) fn card_border(self) -> Style {
        self.card_border
    }

    /// The scrollbar's draggable thumb.
    pub(crate) fn scrollbar_thumb(self) -> Style {
        self.scrollbar_thumb
    }

    /// The scrollbar's track behind the thumb.
    pub(crate) fn scrollbar_track(self) -> Style {
        self.scrollbar_track
    }

    /// The scrollbar's up/down arrow caps. Defaults to the thumb hue unless a
    /// theme sets `scrollbar.arrow`.
    pub(crate) fn scrollbar_arrow(self) -> Style {
        self.scrollbar_arrow
    }

    // --- charts ---

    /// The filled part of count/frequency bars.
    pub(crate) fn chart_bar(self) -> Fill {
        self.bar
    }

    /// The empty remainder of a bar.
    pub(crate) fn chart_track(self) -> Fill {
        self.track
    }

    /// The positive sentiment series.
    pub(crate) fn chart_positive(self) -> Fill {
        self.chart_positive
    }

    /// The neutral sentiment series.
    pub(crate) fn chart_neutral(self) -> Fill {
        self.chart_neutral
    }

    /// The negative sentiment series.
    pub(crate) fn chart_negative(self) -> Fill {
        self.chart_negative
    }

    /// The zero baseline of signed column charts.
    pub(crate) fn chart_baseline(self) -> Style {
        self.chart_baseline
    }

    /// Chart captions and column labels.
    pub(crate) fn chart_label(self) -> Style {
        self.chart_label
    }

    // --- markdown ---

    /// Markdown headings in the entry viewer.
    pub(crate) fn md_heading(self) -> Style {
        self.md_heading
    }

    /// Third-level markdown headings, for themes that fade deeper levels.
    /// (H2 is body ink + bold by the renderer's design.)
    pub(crate) fn md_heading3(self) -> Style {
        self.md_heading3
    }

    /// Markdown links.
    pub(crate) fn md_link(self) -> Style {
        self.md_link
    }

    /// Inline code and code blocks.
    pub(crate) fn md_code(self) -> Style {
        self.md_code
    }

    /// Block quotes.
    pub(crate) fn md_blockquote(self) -> Style {
        self.md_blockquote
    }

    /// Syntax-highlight colors for fenced code blocks.
    pub(crate) fn syntax(self) -> Syntax {
        self.syntax
    }

    // --- glyphs ---

    /// The theme's identity glyphs.
    pub(crate) fn glyphs(self) -> Glyphs {
        self.glyphs
    }

    /// The marker before a selected list row: the theme's glyph if set,
    /// otherwise the chrome's built-in (`●` flat, `>` bordered).
    pub(crate) fn selection_marker(self) -> char {
        self.glyphs.selection_marker.unwrap_or(match self.chrome {
            ChromeStyle::Flat => '●',
            ChromeStyle::Bordered => '>',
        })
    }

    // --- chrome ---

    /// Whether this theme separates surfaces by background or drawn borders.
    pub(crate) fn chrome(self) -> ChromeStyle {
        self.chrome
    }

    /// How strongly the screen dims behind dialogs, `0.0..=1.0`. Zero means
    /// the DIM-modifier fallback.
    pub(crate) fn scrim_strength(self) -> f32 {
        self.scrim
    }

    /// Every style the theme carries, for whole-theme assertions in tests.
    #[cfg(test)]
    fn all_styles(&self) -> Vec<(&'static str, Style)> {
        vec![
            ("text", self.text),
            ("muted", self.muted),
            ("heading", self.heading),
            ("placeholder", self.placeholder),
            ("primary", self.primary),
            ("border", self.border),
            ("border_subtle", self.border_subtle),
            ("border_active", self.border_active),
            ("border_inactive", self.border_inactive),
            ("success", self.success),
            ("warning", self.warning),
            ("error", self.error),
            ("info", self.info),
            ("selection", self.selection),
            ("hover", self.hover),
            ("button", self.button),
            ("key_hint", self.key_hint),
            ("cursor", self.cursor),
            ("cursor_line", self.cursor_line),
            ("scrollbar_thumb", self.scrollbar_thumb),
            ("scrollbar_track", self.scrollbar_track),
            ("chart_positive", self.chart_positive.style),
            ("chart_neutral", self.chart_neutral.style),
            ("chart_negative", self.chart_negative.style),
            ("bar", self.bar.style),
            ("track", self.track.style),
            ("chart_baseline", self.chart_baseline),
            ("chart_label", self.chart_label),
            ("md_heading", self.md_heading),
            ("md_heading3", self.md_heading3),
            ("md_link", self.md_link),
            ("md_code", self.md_code),
            ("md_blockquote", self.md_blockquote),
        ]
    }
}

// --- loading ---

/// The directory holding the user-editable theme files, next to `config.toml`.
pub(crate) fn themes_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("themes")
}

/// Write every bundled theme that isn't on disk yet. Existing files are never
/// touched — user edits win over bundled updates.
pub(crate) fn ensure_bundled(dir: &Path) -> Result<()> {
    for (name, text) in BUNDLED {
        let path = dir.join(format!("{name}.toml"));
        if !path.exists() {
            crate::config::write_toml_atomic(&path, text)
                .with_context(|| format!("materializing bundled theme {}", path.display()))?;
        }
    }
    Ok(())
}

/// Load the named theme, materializing the bundled files first. Any failure
/// (missing file, bad TOML, unknown color) falls back to the built-in
/// [`DEFAULT_THEME`] with a warning on stderr — the app always starts.
pub(crate) fn load(config_path: &Path, name: &str, mode: Mode) -> Theme {
    match try_load(config_path, name, mode) {
        Ok(theme) => theme,
        Err(err) => {
            eprintln!("warning: {err:#}; using the built-in '{DEFAULT_THEME}' theme");
            builtin(DEFAULT_THEME, mode).unwrap_or_else(Theme::terminal_default)
        }
    }
}

/// Load and resolve one theme file. Errors carry the path and token context so
/// a typo in a user file names itself.
pub(crate) fn load_file(path: &Path, mode: Mode) -> Result<Theme> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading theme {}", path.display()))?;
    parse(&text, mode).with_context(|| format!("in theme {}", path.display()))
}

fn try_load(config_path: &Path, name: &str, mode: Mode) -> Result<Theme> {
    let dir = themes_dir(config_path);
    ensure_bundled(&dir)?;
    load_file(&dir.join(format!("{name}.toml")), mode)
}

/// Resolve a bundled theme straight from its embedded text.
fn builtin(name: &str, mode: Mode) -> Option<Theme> {
    let (_, text) = BUNDLED.iter().find(|(n, _)| *n == name)?;
    parse(text, mode).ok()
}
