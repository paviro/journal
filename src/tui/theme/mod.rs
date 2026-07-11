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
    sync::RwLock,
};

/// The bundled themes, embedded so the binary can materialize and fall back to
/// them without touching the network or the repo.
const BUNDLED: [(&str, &str); 10] = [
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
        }
    }

    /// The `Block` border set for a panel, thickened when focused — thickness
    /// is how focus survives monochrome. Ascii has no thick variant; there
    /// focus is carried by the bold border style alone.
    pub(crate) fn block_set(self, focused: bool) -> ratatui::symbols::border::Set<'static> {
        if focused && self != BorderGlyphs::Ascii {
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
    /// The box-drawing set for borders, cards, and table grids (`borders.style`).
    pub(crate) borders: BorderGlyphs,
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
    border: Style,
    border_subtle: Style,
    border_active: Style,
    border_inactive: Style,
    success: Style,
    warning: Style,
    error: Style,
    info: Style,
    selection: Style,
    hover: Style,
    button: Style,
    key_hint: Style,
    cursor: Style,
    cursor_line: Style,
    scrollbar_thumb: Style,
    scrollbar_track: Style,
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

/// The installed theme. `None` until [`install`] runs; readers fall back to
/// [`Theme::terminal_default`], which is what every test exercises.
static THEME: RwLock<Option<Theme>> = RwLock::new(None);

#[cfg(test)]
thread_local! {
    /// Per-thread override so parallel tests can pin a theme without racing
    /// the process-global. Each `#[test]` runs on its own thread, so a set
    /// value never leaks into another test.
    static TEST_THEME: std::cell::Cell<Option<Theme>> = const { std::cell::Cell::new(None) };
}

/// The user's chrome override (`[ui] chrome = "flat"|"bordered"`), applied on
/// top of whatever the active theme declares as its `chrome.style`. `None`
/// (= `default`) follows the theme. Runtime-writable so the theme picker can
/// cycle it with live preview.
#[cfg(not(test))]
static CHROME_OVERRIDE: RwLock<Option<ChromeStyle>> = RwLock::new(None);

#[cfg(test)]
thread_local! {
    /// Per-thread override so parallel tests can pin a chrome without racing
    /// the process-global (mirrors [`TEST_THEME`]).
    static TEST_CHROME_OVERRIDE: std::cell::Cell<Option<ChromeStyle>> =
        const { std::cell::Cell::new(None) };
}

/// The forced chrome style, or `None` when following the theme (`default`).
pub(crate) fn chrome_override() -> Option<ChromeStyle> {
    #[cfg(test)]
    return TEST_CHROME_OVERRIDE.with(std::cell::Cell::get);
    #[cfg(not(test))]
    *CHROME_OVERRIDE.read().expect("chrome override lock")
}

/// Force a chrome style on every theme (`None` = follow the theme). The next
/// frame repaints with it — `theme()` applies it on read.
pub(crate) fn set_chrome_override(style: Option<ChromeStyle>) {
    #[cfg(test)]
    TEST_CHROME_OVERRIDE.with(|cell| cell.set(style));
    #[cfg(not(test))]
    {
        *CHROME_OVERRIDE.write().expect("chrome override lock") = style;
    }
}

/// The current theme, with the chrome override applied. Cheap to call
/// everywhere: `Theme` is `Copy`.
pub(crate) fn theme() -> Theme {
    #[cfg(test)]
    if let Some(mut theme) = TEST_THEME.with(std::cell::Cell::get) {
        if let Some(style) = chrome_override() {
            theme.chrome = style;
        }
        return theme;
    }
    let mut theme = THEME
        .read()
        .expect("theme lock")
        .unwrap_or_else(Theme::terminal_default);
    if let Some(style) = chrome_override() {
        theme.chrome = style;
    }
    theme
}

/// Swap the active theme; the next frame repaints with it. Used at startup,
/// by live reload, and by the theme picker's preview. Under test it swaps the
/// per-thread override instead, so parallel tests can't restyle each other.
pub(crate) fn install(theme: Theme) {
    #[cfg(test)]
    TEST_THEME.with(|cell| cell.set(Some(theme)));
    #[cfg(not(test))]
    {
        *THEME.write().expect("theme lock") = Some(theme);
    }
}

/// The dark/light mode resolved at startup, cached so live reload and the
/// theme picker resolve theme files against the same variant. `Dark` until
/// [`init_from_config`] runs.
static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();

/// The session's resolved dark/light mode.
pub(crate) fn mode() -> Mode {
    MODE.get().copied().unwrap_or(Mode::Dark)
}

/// Detect the mode, then load and install the configured theme. Must run
/// before the terminal enters raw mode / the alternate screen: the `auto`
/// detection talks OSC to the normal screen.
pub(crate) fn init_from_config(config_path: &Path, ui: &crate::config::UiSection) {
    let mode = detect_mode(ui.color_mode);
    let _ = MODE.set(mode);
    set_chrome_override(ui.chrome.forced_style());
    install(load(config_path, &ui.theme, mode));
}

/// Resolve the configured color mode: an explicit setting wins; `auto` asks
/// the terminal for its background (OSC 10/11, with the library's own support
/// heuristic and timeout) and falls back to dark when the answer is unknown.
fn detect_mode(color_mode: crate::config::ColorMode) -> Mode {
    use crate::config::ColorMode;
    match color_mode {
        ColorMode::Dark => Mode::Dark,
        ColorMode::Light => Mode::Light,
        ColorMode::Auto => {
            match terminal_colorsaurus::theme_mode(terminal_colorsaurus::QueryOptions::default()) {
                Ok(terminal_colorsaurus::ThemeMode::Light) => Mode::Light,
                Ok(terminal_colorsaurus::ThemeMode::Dark) | Err(_) => Mode::Dark,
            }
        }
    }
}

/// Pin the theme seen by `theme()` on this test thread.
#[cfg(test)]
pub(crate) fn set_test_theme(theme: Theme) {
    TEST_THEME.with(|cell| cell.set(Some(theme)));
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

    /// The active tab in the tab strip while the panel is focused: accent+bold
    /// on flat chrome (matching focused panel titles — the strip sits in the
    /// title row), selection-styled on bordered chrome so it reads even
    /// without colour. Unfocused it's just bold either way, so it still stands
    /// apart from the muted inactive tabs.
    pub(crate) fn active_tab(self, focused: bool) -> Style {
        if !focused {
            return Style::default().add_modifier(Modifier::BOLD);
        }
        if self.chrome == ChromeStyle::Flat {
            self.primary.add_modifier(Modifier::BOLD)
        } else {
            self.selection.add_modifier(Modifier::BOLD)
        }
    }

    /// A non-active tab.
    pub(crate) fn inactive_tab(self) -> Style {
        self.muted()
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

    /// A recessed box outline — a touch brighter than [`Self::faint_rule`] so
    /// card and panel borders read as present-but-quiet.
    pub(crate) fn card_border(self) -> Style {
        self.border
    }

    /// The scrollbar's draggable thumb.
    pub(crate) fn scrollbar_thumb(self) -> Style {
        self.scrollbar_thumb
    }

    /// The scrollbar's track behind the thumb.
    pub(crate) fn scrollbar_track(self) -> Style {
        self.scrollbar_track
    }

    // --- charts ---

    /// Alias of the count/frequency bar fill's style.
    pub(crate) fn bar_fill(self) -> Style {
        self.bar.style
    }

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
fn ensure_bundled(dir: &Path) -> Result<()> {
    for (name, text) in BUNDLED {
        let path = dir.join(format!("{name}.toml"));
        if !path.exists() {
            crate::config::write_toml_atomic(&path, text)
                .with_context(|| format!("materializing bundled theme {}", path.display()))?;
        }
    }
    Ok(())
}

/// Materialize any missing bundled theme files, so the theme picker lists them
/// even when startup loading hasn't touched the directory yet.
pub(crate) fn ensure_bundled_themes(config_path: &Path) -> Result<()> {
    ensure_bundled(&themes_dir(config_path))
}

/// Load the named theme, materializing the bundled files first. Any failure
/// (missing file, bad TOML, unknown color) falls back to the built-in
/// [`DEFAULT_THEME`] with a warning on stderr — the app always starts.
#[allow(dead_code)] // wired up when startup loading lands
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
