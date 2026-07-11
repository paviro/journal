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

use anyhow::{Context, Result, anyhow, bail};
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::RwLock,
};

/// The bundled themes, embedded so the binary can materialize and fall back to
/// them without touching the network or the repo.
const BUNDLED: [(&str, &str); 10] = [
    ("journal", include_str!("themes/journal.toml")),
    ("classic", include_str!("themes/classic.toml")),
    ("e-ink", include_str!("themes/e-ink.toml")),
    ("blossom", include_str!("themes/blossom.toml")),
    ("fjord", include_str!("themes/fjord.toml")),
    ("grove", include_str!("themes/grove.toml")),
    ("tokyonight", include_str!("themes/tokyonight.toml")),
    ("catppuccin", include_str!("themes/catppuccin.toml")),
    ("matcha", include_str!("themes/matcha.toml")),
    ("rose-pine", include_str!("themes/rose-pine.toml")),
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

fn parse(text: &str, mode: Mode) -> Result<Theme> {
    let file: ThemeFile = toml::from_str(text).context("parsing theme TOML")?;
    file.resolve(mode)
}

// --- TOML schema ---

/// A color position in a theme file: a single color or a `{ dark, light }`
/// pair. Strings are palette names first, then `Color::from_str` forms
/// ("cyan", "#rrggbb", "244"), with "none" meaning the terminal default.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ColorSpec {
    Single(String),
    Variant { dark: String, light: String },
}

impl ColorSpec {
    fn pick(&self, mode: Mode) -> &str {
        match self {
            ColorSpec::Single(name) => name,
            ColorSpec::Variant { dark, light } => match mode {
                Mode::Dark => dark,
                Mode::Light => light,
            },
        }
    }

    fn resolve(&self, mode: Mode, palette: &Palette, token: &str) -> Result<Color> {
        let name = self.pick(mode);
        // Palette entries may themselves be dark/light variants, but not
        // reference other entries — one level keeps lookups cycle-free.
        let name = palette.get(name).map_or(name, |entry| entry.pick(mode));
        parse_color(name).with_context(|| format!("in `{token}`"))
    }
}

type Palette = BTreeMap<String, ColorSpec>;

/// A color nudged one visual step off the background it sits on — toward
/// white on dark backgrounds, toward black on light. Non-RGB colors can't
/// blend and pass through unchanged (the terminal-default look stays inert).
fn lift(color: Color, mode: Mode) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let toward: f32 = match mode {
        Mode::Dark => 255.0,
        Mode::Light => 0.0,
    };
    let blend = |c: u8| (f32::from(c) + (toward - f32::from(c)) * 0.10) as u8;
    Color::Rgb(blend(r), blend(g), blend(b))
}

fn parse_color(name: &str) -> Result<Color> {
    if name.eq_ignore_ascii_case("none") {
        return Ok(Color::Reset);
    }
    Color::from_str(name).map_err(|_| {
        anyhow!("unrecognized color '{name}' (expected a color name, \"#rrggbb\", \"0\"-\"255\", \"none\", or a [palette] entry)")
    })
}

/// A style position: `fg`/`bg` colors plus the modifiers a theme may set.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StyleSpec {
    fg: Option<ColorSpec>,
    bg: Option<ColorSpec>,
    bold: bool,
    dim: bool,
    reversed: bool,
    underlined: bool,
}

impl StyleSpec {
    fn resolve(&self, mode: Mode, palette: &Palette, token: &str) -> Result<Style> {
        let mut style = Style::default();
        if let Some(fg) = &self.fg {
            style = style.fg(fg.resolve(mode, palette, token)?);
        }
        if let Some(bg) = &self.bg {
            style = style.bg(bg.resolve(mode, palette, token)?);
        }
        if self.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.dim {
            style = style.add_modifier(Modifier::DIM);
        }
        if self.reversed {
            style = style.add_modifier(Modifier::REVERSED);
        }
        if self.underlined {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        Ok(style)
    }
}

/// A token that accepts either a bare color (used as the foreground) or a full
/// style table.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TokenSpec {
    Color(ColorSpec),
    Style(StyleSpec),
}

impl TokenSpec {
    fn resolve(&self, mode: Mode, palette: &Palette, token: &str) -> Result<Style> {
        match self {
            TokenSpec::Color(color) => {
                Ok(Style::default().fg(color.resolve(mode, palette, token)?))
            }
            TokenSpec::Style(style) => style.resolve(mode, palette, token),
        }
    }
}

/// A chart fill: the glyph plus an optional color. The meaning-carrying
/// modifiers (bold on signed series, dim on neutral/track) are added in code.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FillSpec {
    glyph: String,
    color: Option<ColorSpec>,
}

impl FillSpec {
    fn resolve(&self, mode: Mode, palette: &Palette, token: &str) -> Result<Fill> {
        let glyph = parse_glyph(&self.glyph, token)?;
        let mut style = Style::default();
        if let Some(color) = &self.color {
            style = style.fg(color.resolve(mode, palette, token)?);
        }
        Ok(Fill { glyph, style })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ThemeFile {
    chrome: ChromeSection,
    palette: Palette,
    surfaces: SurfacesSection,
    text: TextSection,
    accents: AccentsSection,
    status: StatusSection,
    borders: BordersSection,
    interaction: InteractionSection,
    scrollbar: ScrollbarSection,
    charts: ChartsSection,
    markdown: MarkdownSection,
    glyphs: GlyphsSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChromeSection {
    /// The theme's preferred chrome — a preference, not a mandate, because the
    /// `[ui] chrome` setting can force flat/bordered on any theme.
    style: ChromeStyle,
    scrim: f32,
}

impl Default for ChromeSection {
    fn default() -> Self {
        Self {
            style: ChromeStyle::Bordered,
            scrim: 0.0,
        }
    }
}

/// The background layers the UI is built from, base to top.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SurfacesSection {
    background: Option<ColorSpec>,
    panel: Option<ColorSpec>,
    dialog: Option<ColorSpec>,
    element: Option<ColorSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TextSection {
    body: Option<TokenSpec>,
    muted: Option<TokenSpec>,
    heading: Option<TokenSpec>,
    placeholder: Option<TokenSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AccentsSection {
    primary: Option<TokenSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StatusSection {
    success: Option<TokenSpec>,
    warning: Option<TokenSpec>,
    error: Option<TokenSpec>,
    info: Option<TokenSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct BordersSection {
    /// The box-drawing character set every border is drawn with.
    style: Option<BorderGlyphs>,
    normal: Option<TokenSpec>,
    subtle: Option<TokenSpec>,
    focused: Option<TokenSpec>,
    unfocused: Option<TokenSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct InteractionSection {
    selection: Option<StyleSpec>,
    hover: Option<StyleSpec>,
    button: Option<StyleSpec>,
    key_hint: Option<StyleSpec>,
    cursor: Option<StyleSpec>,
    cursor_line: Option<StyleSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ScrollbarSection {
    thumb: Option<TokenSpec>,
    track: Option<TokenSpec>,
}

/// The zero baseline of signed column charts: glyph and color together.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct BaselineSpec {
    glyph: Option<String>,
    color: Option<ColorSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChartsSection {
    positive: Option<FillSpec>,
    neutral: Option<FillSpec>,
    negative: Option<FillSpec>,
    bar: Option<FillSpec>,
    track: Option<FillSpec>,
    baseline: BaselineSpec,
    label: Option<TokenSpec>,
    groove: Option<String>,
    bar_center: Option<String>,
    mood_stroke: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MarkdownSection {
    heading: Option<TokenSpec>,
    heading3: Option<TokenSpec>,
    link: Option<TokenSpec>,
    code: Option<TokenSpec>,
    blockquote: Option<TokenSpec>,
    syntax: SyntaxSection,
}

/// Syntax-highlight colors for fenced code blocks, one key per category the
/// markdown renderer distinguishes. Omitted categories render as plain code —
/// an empty table is exactly the classic un-highlighted look.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SyntaxSection {
    comment: Option<ColorSpec>,
    keyword: Option<ColorSpec>,
    string: Option<ColorSpec>,
    string_escape: Option<ColorSpec>,
    number: Option<ColorSpec>,
    constant: Option<ColorSpec>,
    function: Option<ColorSpec>,
    r#type: Option<ColorSpec>,
    variable: Option<ColorSpec>,
    property: Option<ColorSpec>,
    operator: Option<ColorSpec>,
    punctuation: Option<ColorSpec>,
    attribute: Option<ColorSpec>,
    tag: Option<ColorSpec>,
    label: Option<ColorSpec>,
    error: Option<ColorSpec>,
}

impl SyntaxSection {
    fn resolve(&self, mode: Mode, palette: &Palette) -> Result<Syntax> {
        let color = |spec: &Option<ColorSpec>, token: &str| -> Result<Color> {
            spec.as_ref()
                .map_or(Ok(Color::Reset), |spec| spec.resolve(mode, palette, token))
        };
        Ok(Syntax {
            comment: color(&self.comment, "markdown.syntax.comment")?,
            keyword: color(&self.keyword, "markdown.syntax.keyword")?,
            string: color(&self.string, "markdown.syntax.string")?,
            string_escape: color(&self.string_escape, "markdown.syntax.string_escape")?,
            number: color(&self.number, "markdown.syntax.number")?,
            constant: color(&self.constant, "markdown.syntax.constant")?,
            function: color(&self.function, "markdown.syntax.function")?,
            r#type: color(&self.r#type, "markdown.syntax.type")?,
            variable: color(&self.variable, "markdown.syntax.variable")?,
            property: color(&self.property, "markdown.syntax.property")?,
            operator: color(&self.operator, "markdown.syntax.operator")?,
            punctuation: color(&self.punctuation, "markdown.syntax.punctuation")?,
            attribute: color(&self.attribute, "markdown.syntax.attribute")?,
            tag: color(&self.tag, "markdown.syntax.tag")?,
            label: color(&self.label, "markdown.syntax.label")?,
            error: color(&self.error, "markdown.syntax.error")?,
        })
    }
}

/// The theme's identity glyphs — the ones that aren't chart furniture (those
/// live in `[charts]`) or the border set (`borders.style`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GlyphsSection {
    selection_marker: Option<String>,
    focus_stripe: Option<String>,
    toast_edge: Option<String>,
    tab_separator: Option<String>,
    divider: Option<String>,
}

/// A single-character glyph value.
fn parse_glyph(spec: &str, token: &str) -> Result<char> {
    let mut chars = spec.chars();
    let (Some(glyph), None) = (chars.next(), chars.next()) else {
        bail!("glyph for `{token}` must be exactly one character, got {spec:?}");
    };
    Ok(glyph)
}

impl ThemeFile {
    /// Flatten the file into a [`Theme`] for one [`Mode`]. Omitted tokens fall
    /// back to the classic look, so an empty file *is* `classic.toml`.
    fn resolve(&self, mode: Mode) -> Result<Theme> {
        let palette = &self.palette;
        let color = |spec: &Option<ColorSpec>, default: Color, token: &str| -> Result<Color> {
            spec.as_ref()
                .map_or(Ok(default), |spec| spec.resolve(mode, palette, token))
        };
        let style = |spec: &Option<TokenSpec>, default: Style, token: &str| -> Result<Style> {
            spec.as_ref()
                .map_or(Ok(default), |spec| spec.resolve(mode, palette, token))
        };
        let fill = |spec: &Option<FillSpec>,
                    glyph: char,
                    default: Style,
                    carries: Modifier,
                    token: &str|
         -> Result<Fill> {
            let fill = match spec {
                Some(spec) => spec.resolve(mode, palette, token)?,
                None => Fill {
                    glyph,
                    style: default,
                },
            };
            Ok(Fill {
                glyph: fill.glyph,
                style: fill.style.add_modifier(carries),
            })
        };

        let surfaces = &self.surfaces;
        let bg = color(&surfaces.background, Color::Reset, "surfaces.background")?;
        let panel = color(&surfaces.panel, bg, "surfaces.panel")?;
        let dialog = color(&surfaces.dialog, panel, "surfaces.dialog")?;
        let element = color(&surfaces.element, panel, "surfaces.element")?;
        let text = style(&self.text.body, Style::default(), "text.body")?;
        let muted = style(&self.text.muted, Style::default(), "text.muted")?;
        let heading = style(&self.text.heading, text, "text.heading")?;
        let placeholder = style(&self.text.placeholder, muted, "text.placeholder")?;
        let primary = style(
            &self.accents.primary,
            Style::default().fg(Color::Cyan),
            "accents.primary",
        )?;
        let borders = &self.borders;
        let border = style(
            &borders.normal,
            Style::default().fg(Color::Indexed(244)),
            "borders.normal",
        )?;
        let border_subtle = style(
            &borders.subtle,
            Style::default().fg(Color::Indexed(240)),
            "borders.subtle",
        )?;
        let border_active = style(&borders.focused, Style::default(), "borders.focused")?;
        let border_inactive = style(&borders.unfocused, Style::default(), "borders.unfocused")?;
        // The thumb inherits the focused-border hue (it marks the scrollable,
        // interactable panel); the track stays terminal-default quiet.
        let scrollbar_thumb = style(&self.scrollbar.thumb, border_active, "scrollbar.thumb")?;
        let scrollbar_track = style(&self.scrollbar.track, Style::default(), "scrollbar.track")?;

        let interaction = &self.interaction;
        let selection = match &interaction.selection {
            Some(spec) => {
                if spec.bg.is_some() && spec.fg.is_none() {
                    bail!(
                        "`interaction.selection` sets a bg without an fg; pick a readable \
                         foreground explicitly"
                    );
                }
                spec.resolve(mode, palette, "interaction.selection")?
            }
            None => Style::default().add_modifier(Modifier::REVERSED),
        };
        // Unlike selection, a bg-only hover is fine: it layers under the
        // row's existing foreground instead of replacing it. The default
        // *lifts* the element surface rather than reusing it: element is what
        // cards/chips already sit on, so a same-color hover would be invisible
        // exactly where hover matters most — and theme files written before
        // the token existed (never overwritten on upgrade) hit this default.
        let hover = match &interaction.hover {
            Some(spec) => spec.resolve(mode, palette, "interaction.hover")?,
            None => Style::default().bg(lift(element, mode)),
        };
        // Buttons are selection-colored unless a theme splits them; the same
        // readable-fill rule applies.
        let button = match &interaction.button {
            Some(spec) => {
                if spec.bg.is_some() && spec.fg.is_none() {
                    bail!(
                        "`interaction.button` sets a bg without an fg; pick a readable \
                         foreground explicitly"
                    );
                }
                spec.resolve(mode, palette, "interaction.button")?
            }
            None => selection,
        };
        let key_hint = match &interaction.key_hint {
            Some(spec) => spec.resolve(mode, palette, "interaction.key_hint")?,
            None => Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        };
        // Both default to "no styling": the terminal's own cursor shape and no
        // cursor-line highlight, which is what the app always did.
        let cursor = match &interaction.cursor {
            Some(spec) => spec.resolve(mode, palette, "interaction.cursor")?,
            None => Style::default(),
        };
        let cursor_line = match &interaction.cursor_line {
            Some(spec) => spec.resolve(mode, palette, "interaction.cursor_line")?,
            None => Style::default(),
        };

        let charts = &self.charts;
        // The classic chart palette as fill defaults; bold/dim are the
        // monochrome contract and always come from code.
        let chart_positive = fill(
            &charts.positive,
            '▓',
            Style::default().fg(Color::Green),
            Modifier::BOLD,
            "charts.positive",
        )?;
        let chart_neutral = fill(
            &charts.neutral,
            '▓',
            Style::default(),
            Modifier::DIM,
            "charts.neutral",
        )?;
        let chart_negative = fill(
            &charts.negative,
            '▓',
            Style::default().fg(Color::Red),
            Modifier::BOLD,
            "charts.negative",
        )?;
        let bar = fill(
            &charts.bar,
            '▓',
            Style::default().fg(Color::Cyan),
            Modifier::empty(),
            "charts.bar",
        )?;
        let track = fill(
            &charts.track,
            '░',
            Style::default(),
            Modifier::DIM,
            "charts.track",
        )?;
        // Chart furniture defaults to the muted ink so charts read as they
        // always have when a theme doesn't restyle them.
        let chart_furniture = muted.add_modifier(Modifier::DIM);
        let chart_baseline = match &charts.baseline.color {
            Some(spec) => {
                Style::default().fg(spec.resolve(mode, palette, "charts.baseline.color")?)
            }
            None => chart_furniture,
        };
        let chart_label = style(&charts.label, chart_furniture, "charts.label")?;

        let markdown = &self.markdown;
        let md_heading = style(&markdown.heading, Style::default(), "markdown.heading")?;
        let md_heading3 = style(&markdown.heading3, md_heading, "markdown.heading3")?;
        let md_link = style(
            &markdown.link,
            Style::default().add_modifier(Modifier::UNDERLINED),
            "markdown.link",
        )?;
        let md_code = style(&markdown.code, Style::default(), "markdown.code")?;
        let md_blockquote = style(
            &markdown.blockquote,
            Style::default(),
            "markdown.blockquote",
        )?;

        if !(0.0..=1.0).contains(&self.chrome.scrim) {
            bail!("`chrome.scrim` must be between 0.0 and 1.0");
        }

        let glyph = |spec: &Option<String>, default: char, token: &str| -> Result<char> {
            spec.as_deref()
                .map_or(Ok(default), |spec| parse_glyph(spec, token))
        };
        let glyphs = Glyphs {
            selection_marker: self
                .glyphs
                .selection_marker
                .as_deref()
                .map(|spec| parse_glyph(spec, "glyphs.selection_marker"))
                .transpose()?,
            focus_stripe: glyph(&self.glyphs.focus_stripe, '┃', "glyphs.focus_stripe")?,
            toast_edge: glyph(&self.glyphs.toast_edge, '┃', "glyphs.toast_edge")?,
            tab_separator: glyph(&self.glyphs.tab_separator, '·', "glyphs.tab_separator")?,
            divider: glyph(&self.glyphs.divider, '━', "glyphs.divider")?,
            chart_baseline: glyph(&charts.baseline.glyph, '┈', "charts.baseline.glyph")?,
            chart_groove: glyph(&charts.groove, '·', "charts.groove")?,
            bar_center: glyph(&charts.bar_center, '│', "charts.bar_center")?,
            mood_fill: glyph(&charts.mood_stroke, '─', "charts.mood_stroke")?,
            borders: borders.style.unwrap_or_default(),
        };

        let status = &self.status;
        Ok(Theme {
            bg,
            panel,
            dialog,
            element,
            text,
            muted,
            heading,
            placeholder,
            primary,
            border,
            border_subtle,
            border_active,
            border_inactive,
            success: style(
                &status.success,
                Style::default().fg(Color::Green),
                "status.success",
            )?,
            warning: style(
                &status.warning,
                Style::default().fg(Color::Yellow),
                "status.warning",
            )?,
            error: style(
                &status.error,
                Style::default().fg(Color::Red),
                "status.error",
            )?,
            info: style(
                &status.info,
                Style::default().fg(Color::Blue),
                "status.info",
            )?,
            selection,
            hover,
            button,
            key_hint,
            cursor,
            cursor_line,
            scrollbar_thumb,
            scrollbar_track,
            chart_positive,
            chart_neutral,
            chart_negative,
            bar,
            track,
            chart_baseline,
            chart_label,
            md_heading,
            md_heading3,
            md_link,
            md_code,
            md_blockquote,
            syntax: markdown.syntax.resolve(mode, palette)?,
            glyphs,
            chrome: self.chrome.style,
            scrim: self.chrome.scrim,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn bundled(name: &str) -> &'static str {
        BUNDLED
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, text)| *text)
            .expect("bundled theme exists")
    }

    #[test]
    fn every_bundled_theme_parses_in_both_modes() {
        for (name, text) in BUNDLED {
            for mode in [Mode::Dark, Mode::Light] {
                parse(text, mode).unwrap_or_else(|err| {
                    panic!("bundled theme '{name}' failed to resolve ({mode:?}): {err:#}")
                });
            }
        }
    }

    #[test]
    fn chrome_override_wins_over_the_theme_default() {
        set_test_theme(test_flat_theme());
        assert_eq!(theme().chrome(), ChromeStyle::Flat);
        set_chrome_override(Some(ChromeStyle::Bordered));
        assert_eq!(theme().chrome(), ChromeStyle::Bordered, "override ignored");
        set_chrome_override(None);
        assert_eq!(
            theme().chrome(),
            ChromeStyle::Flat,
            "auto must follow the theme"
        );
    }

    #[test]
    fn default_hover_lifts_the_element_surface() {
        // Theme files written before the hover token existed (materialized
        // copies are never overwritten) must still get a visible hover: the
        // default nudges element toward white (dark) / black (light).
        let text =
            "[surfaces]\nbackground = \"#101010\"\npanel = \"#181818\"\nelement = \"#202020\"";
        let dark = parse(text, Mode::Dark).unwrap();
        assert_eq!(dark.hover().bg, Some(Color::Rgb(0x36, 0x36, 0x36)));
        let light = parse(
            "[surfaces]\nbackground = \"#f0f0f0\"\npanel = \"#e8e8e8\"\nelement = \"#e0e0e0\"",
            Mode::Light,
        )
        .unwrap();
        assert_eq!(light.hover().bg, Some(Color::Rgb(0xc9, 0xc9, 0xc9)));
    }

    #[test]
    fn new_tokens_chain_to_their_parents() {
        // Each new token inherits its parent when omitted, so themes written
        // before the tokens existed keep rendering as they did.
        let theme = parse(
            "[text]\n\
             body = \"#aabbcc\"\n\
             muted = \"#334455\"\n\
             [interaction]\n\
             selection = { fg = \"#000000\", bg = \"#ffffff\" }\n\
             [borders]\n\
             focused = \"#606060\"\n\
             [markdown]\n\
             heading = \"#56b6b0\"",
            Mode::Dark,
        )
        .unwrap();
        assert_eq!(
            theme.heading(),
            Style::default()
                .fg(Color::Rgb(0xaa, 0xbb, 0xcc))
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            theme.placeholder(),
            Style::default()
                .fg(Color::Rgb(0x33, 0x44, 0x55))
                .add_modifier(Modifier::DIM)
        );
        assert_eq!(theme.button(), theme.selection());
        assert_eq!(
            theme.scrollbar_thumb(),
            Style::default().fg(Color::Rgb(0x60, 0x60, 0x60))
        );
        assert_eq!(theme.scrollbar_track(), Style::default());
        assert_eq!(theme.md_heading3(), theme.md_heading());
        // The editor tokens default to "no styling".
        assert_eq!(theme.cursor(), Style::default());
        assert_eq!(theme.cursor_line(), Style::default());
    }

    #[test]
    fn new_tokens_resolve_explicit_values() {
        let theme = parse(
            "[text]\n\
             heading = \"#112233\"\n\
             placeholder = \"#445566\"\n\
             [interaction]\n\
             button = { fg = \"#000000\", bg = \"#aabbcc\" }\n\
             cursor = { reversed = true }\n\
             cursor_line = { bg = \"#181818\" }\n\
             [scrollbar]\n\
             thumb = \"#778899\"\n\
             track = \"#223344\"\n\
             [markdown]\n\
             heading3 = \"#556677\"",
            Mode::Dark,
        )
        .unwrap();
        assert_eq!(theme.heading().fg, Some(Color::Rgb(0x11, 0x22, 0x33)));
        assert_eq!(theme.placeholder().fg, Some(Color::Rgb(0x44, 0x55, 0x66)));
        assert_eq!(theme.button().bg, Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
        assert!(theme.cursor().add_modifier.contains(Modifier::REVERSED));
        assert_eq!(theme.cursor_line().bg, Some(Color::Rgb(0x18, 0x18, 0x18)));
        assert_eq!(
            theme.scrollbar_thumb().fg,
            Some(Color::Rgb(0x77, 0x88, 0x99))
        );
        assert_eq!(
            theme.scrollbar_track().fg,
            Some(Color::Rgb(0x22, 0x33, 0x44))
        );
        assert_eq!(theme.md_heading3().fg, Some(Color::Rgb(0x55, 0x66, 0x77)));
    }

    #[test]
    fn button_rejects_bg_without_fg() {
        let err = parse("[interaction]\nbutton = { bg = \"#aabbcc\" }", Mode::Dark).unwrap_err();
        assert!(err.to_string().contains("interaction.button"), "{err:#}");
    }

    #[test]
    fn glyphs_resolve_and_default() {
        let theme = parse(
            "[borders]\nstyle = \"rounded\"\n[glyphs]\nselection_marker = \"▶\"\nfocus_stripe = \"█\"",
            Mode::Dark,
        )
        .unwrap();
        assert_eq!(theme.selection_marker(), '▶');
        assert_eq!(theme.glyphs().focus_stripe, '█');
        assert_eq!(theme.glyphs().borders, BorderGlyphs::Rounded);
        // Defaults untouched by a partial section.
        assert_eq!(theme.glyphs().toast_edge, '┃');
        assert_eq!(theme.glyphs().divider, '━');

        // With no marker set, the selection marker follows the chrome.
        let default = Theme::terminal_default();
        assert_eq!(default.glyphs().selection_marker, None);
        assert_eq!(default.selection_marker(), '>');
        let mut flat = default;
        flat.chrome = ChromeStyle::Flat;
        assert_eq!(flat.selection_marker(), '●');
        assert_eq!(default.glyphs().borders, BorderGlyphs::Plain);
    }

    #[test]
    fn glyph_tokens_must_be_one_character() {
        let err = parse("[glyphs]\nfocus_stripe = \"ab\"", Mode::Dark).unwrap_err();
        assert!(err.to_string().contains("glyphs.focus_stripe"), "{err:#}");
    }

    #[test]
    fn chart_baseline_merges_glyph_and_color() {
        let theme = parse(
            "[charts]\nbaseline = { glyph = \"╌\", color = \"#123456\" }",
            Mode::Dark,
        )
        .unwrap();
        assert_eq!(theme.glyphs().chart_baseline, '╌');
        assert_eq!(
            theme.chart_baseline(),
            Style::default().fg(Color::Rgb(0x12, 0x34, 0x56))
        );
        // Each half keeps its default when the other is set alone.
        let glyph_only = parse("[charts]\nbaseline = { glyph = \"╌\" }", Mode::Dark).unwrap();
        assert_eq!(glyph_only.glyphs().chart_baseline, '╌');
        assert_eq!(
            glyph_only.chart_baseline(),
            Style::default().add_modifier(Modifier::DIM)
        );
        let color_only = parse("[charts]\nbaseline = { color = \"#123456\" }", Mode::Dark).unwrap();
        assert_eq!(color_only.glyphs().chart_baseline, '┈');
        assert_eq!(
            color_only.chart_baseline().fg,
            Some(Color::Rgb(0x12, 0x34, 0x56))
        );
    }

    #[test]
    fn chart_glyphs_live_in_the_charts_section() {
        let theme = parse(
            "[charts]\ngroove = \"‥\"\nbar_center = \"┋\"\nmood_stroke = \"═\"",
            Mode::Dark,
        )
        .unwrap();
        assert_eq!(theme.glyphs().chart_groove, '‥');
        assert_eq!(theme.glyphs().bar_center, '┋');
        assert_eq!(theme.glyphs().mood_fill, '═');
        // Defaults untouched by a partial section.
        let bare = parse("", Mode::Dark).unwrap();
        assert_eq!(bare.glyphs().chart_groove, '·');
        assert_eq!(bare.glyphs().bar_center, '│');
        assert_eq!(bare.glyphs().mood_fill, '─');
    }

    #[test]
    fn border_glyph_sets_cover_focus_and_ascii() {
        // A themed set thickens for focus; ascii has no thick variant, so a
        // focused ascii panel keeps its own corners (focus rides on the style).
        assert_eq!(BorderGlyphs::Rounded.block_set(false).top_left, "╭");
        assert_eq!(BorderGlyphs::Rounded.block_set(true).top_left, "┏");
        assert_eq!(BorderGlyphs::Ascii.block_set(false).top_left, "+");
        assert_eq!(BorderGlyphs::Ascii.block_set(true).top_left, "+");
        assert_eq!(BorderGlyphs::Ascii.line_set().cross, "+");
    }

    #[test]
    fn syntax_colors_resolve_and_default_to_reset() {
        let theme = parse(
            "[markdown.syntax]\nkeyword = \"#fab283\"\nstring = \"green\"",
            Mode::Dark,
        )
        .unwrap();
        assert_eq!(theme.syntax().keyword, Color::Rgb(0xfa, 0xb2, 0x83));
        assert_eq!(theme.syntax().string, Color::Green);
        // Unset categories stay plain, so classic code blocks don't change.
        assert_eq!(theme.syntax().comment, Color::Reset);
        assert_eq!(Theme::terminal_default().syntax().keyword, Color::Reset);
    }

    #[test]
    fn border_inactive_resolves_and_defaults_to_terminal_ink() {
        let themed = parse("[borders]\nunfocused = \"#3c3c3c\"", Mode::Dark).unwrap();
        assert_eq!(
            themed.inactive_border(),
            Style::default().fg(Color::Rgb(0x3c, 0x3c, 0x3c))
        );
        // Theme files from before the token existed keep the classic look.
        let bare = parse("", Mode::Dark).unwrap();
        assert_eq!(bare.inactive_border(), Style::default());
    }

    #[test]
    fn dialog_defaults_to_panel_for_existing_theme_files() {
        let theme = parse(
            "[surfaces]\nbackground = \"#101010\"\npanel = \"#181818\"",
            Mode::Dark,
        )
        .unwrap();
        assert_eq!(theme.dialog, theme.panel);
    }

    #[test]
    fn flat_bundled_themes_split_dialogs_from_panels() {
        for (name, text) in BUNDLED {
            for mode in [Mode::Dark, Mode::Light] {
                let theme = parse(text, mode).unwrap();
                if theme.chrome == ChromeStyle::Flat {
                    assert_ne!(
                        theme.dialog, theme.panel,
                        "'{name}' dialog matches panel ({mode:?})"
                    );
                    assert_ne!(
                        theme.dialog, theme.element,
                        "'{name}' dialog matches element ({mode:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn every_bundled_theme_clears_the_contrast_floor() {
        // A cheap "renders acceptably in both modes" floor: selection must
        // never smear same-on-same, and body text must never dissolve into
        // the background.
        for (name, text) in BUNDLED {
            for mode in [Mode::Dark, Mode::Light] {
                let theme = parse(text, mode).unwrap();
                match (theme.selection.fg, theme.selection.bg) {
                    (Some(fg), Some(bg)) => {
                        assert_ne!(fg, bg, "'{name}' selection is same-on-same ({mode:?})")
                    }
                    // Without pinned colors the inversion must carry contrast.
                    _ => assert!(
                        theme.selection.add_modifier.contains(Modifier::REVERSED),
                        "'{name}' selection has neither contrast colors nor inversion ({mode:?})"
                    ),
                }
                if let Some(fg) = theme.text.fg {
                    assert_ne!(fg, theme.bg, "'{name}' text matches its bg ({mode:?})");
                }
            }
        }
    }

    #[test]
    fn classic_is_the_builtin_fallback() {
        // classic.toml is the living spec for `terminal_default()`: the two
        // must never drift, in either mode (classic has no variant colors).
        for mode in [Mode::Dark, Mode::Light] {
            assert_eq!(
                parse(bundled("classic"), mode).unwrap(),
                Theme::terminal_default(),
                "classic.toml drifted from Theme::terminal_default ({mode:?})"
            );
        }
    }

    #[test]
    fn terminal_default_matches_the_original_styles() {
        // The pre-theme-engine styles, pinned so the whole render test suite
        // (which never installs a theme) keeps exercising the original look.
        let theme = Theme::terminal_default();
        assert_eq!(
            theme.heading(),
            Style::default().add_modifier(Modifier::BOLD)
        );
        assert_eq!(theme.muted(), Style::default().add_modifier(Modifier::DIM));
        assert_eq!(theme.primary(), Style::default().fg(Color::Cyan));
        assert_eq!(theme.bar_fill(), Style::default().fg(Color::Cyan));
        assert_eq!(
            theme.positive(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            theme.negative(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        );
        assert_eq!(theme.neutral(), Style::default());
        assert_eq!(
            theme.active_tab(true),
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        );
        assert_eq!(
            theme.active_tab(false),
            Style::default().add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            theme.inactive_tab(),
            Style::default().add_modifier(Modifier::DIM)
        );
        assert_eq!(
            theme.focus_border(),
            Style::default().add_modifier(Modifier::BOLD)
        );
        // Unfocused panels and dialog frames keep the terminal-default ink the
        // app always drew them with.
        assert_eq!(theme.inactive_border(), Style::default());
        assert_eq!(theme.dialog_border(), Style::default());
        assert_eq!(theme.faint_rule(), Style::default().fg(Color::Indexed(240)));
        assert_eq!(
            theme.card_border(),
            Style::default().fg(Color::Indexed(244))
        );
        assert_eq!(
            theme.selection(),
            Style::default().add_modifier(Modifier::REVERSED)
        );
        assert_eq!(
            theme.key_hint(),
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        );
        // Hover inherits the element surface, which is the terminal default
        // here — an invisible hover, keeping the classic look inert.
        assert_eq!(theme.hover(), Style::default().bg(Color::Reset));
        // The pass-2 tokens: buttons ride selection, headings ride text,
        // placeholders ride muted, the editor and scrollbars stay unstyled —
        // exactly the pre-token rendering.
        assert_eq!(theme.button(), theme.selection());
        assert_eq!(
            theme.heading(),
            Style::default().add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            theme.placeholder(),
            Style::default().add_modifier(Modifier::DIM)
        );
        assert_eq!(theme.cursor(), Style::default());
        assert_eq!(theme.cursor_line(), Style::default());
        assert_eq!(theme.scrollbar_thumb(), Style::default());
        assert_eq!(theme.scrollbar_track(), Style::default());
        assert_eq!(theme.chrome(), ChromeStyle::Bordered);
        assert_eq!(theme.scrim_strength(), 0.0);
    }

    #[test]
    fn signed_distinguishes_positive_from_negative() {
        let theme = theme();
        assert_ne!(theme.signed(1.0), theme.signed(-1.0));
    }

    #[test]
    fn eink_is_monochrome_high_contrast_in_both_modes() {
        let ink_or_paper =
            |color: Color| color == Color::Rgb(0, 0, 0) || color == Color::Rgb(255, 255, 255);
        for mode in [Mode::Dark, Mode::Light] {
            let theme = parse(bundled("e-ink"), mode).unwrap();
            for (name, style) in theme.all_styles() {
                for color in [style.fg, style.bg].into_iter().flatten() {
                    assert!(
                        ink_or_paper(color),
                        "e-ink `{name}` uses non-monochrome {color:?} ({mode:?})"
                    );
                }
            }
            for color in [theme.bg, theme.panel, theme.element] {
                assert!(ink_or_paper(color), "e-ink surface {color:?} ({mode:?})");
            }
            assert_ne!(
                theme.dialog, theme.bg,
                "e-ink dialog should lift off the main background ({mode:?})"
            );
            assert_eq!(
                theme.dialog,
                match mode {
                    Mode::Dark => Color::Rgb(0x1a, 0x1a, 0x1a),
                    Mode::Light => Color::Rgb(0xf2, 0xf2, 0xf2),
                }
            );
            // Series identity must survive without hue: three distinct glyphs.
            let glyphs = [
                theme.chart_positive.glyph,
                theme.chart_neutral.glyph,
                theme.chart_negative.glyph,
            ];
            assert_eq!(
                glyphs.len(),
                glyphs
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                "e-ink chart series share a glyph"
            );
            // Selection must be a true inversion, not a same-on-same smear.
            assert_ne!(theme.selection.fg, theme.selection.bg);
            // Signed values keep their weight with color stripped.
            assert!(theme.positive().add_modifier.contains(Modifier::BOLD));
            assert!(theme.negative().add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn journal_resolves_variants_by_mode() {
        let dark = parse(bundled("journal"), Mode::Dark).unwrap();
        let light = parse(bundled("journal"), Mode::Light).unwrap();
        assert_eq!(dark.bg, Color::Rgb(0x0a, 0x0a, 0x0a));
        assert_eq!(light.bg, Color::Rgb(0xfc, 0xfc, 0xfc));
        assert_eq!(dark.primary.fg, Some(Color::Rgb(0x56, 0xb6, 0xb0)));
        assert_eq!(light.primary.fg, Some(Color::Rgb(0x15, 0x7d, 0x76)));
        assert_eq!(dark.chrome, ChromeStyle::Flat);
        assert!(dark.scrim > 0.0);
    }

    #[test]
    fn palette_entries_resolve_including_variants() {
        let theme = parse(
            r##"
            [palette]
            splash = { dark = "#102030", light = "#e0e0e0" }
            flat = "#445566"

            [accents]
            primary = "splash"

            [status]
            info = "flat"
            "##,
            Mode::Light,
        )
        .unwrap();
        assert_eq!(theme.primary.fg, Some(Color::Rgb(0xe0, 0xe0, 0xe0)));
        assert_eq!(theme.info.fg, Some(Color::Rgb(0x44, 0x55, 0x66)));
    }

    #[test]
    fn color_forms_parse() {
        assert_eq!(parse_color("none").unwrap(), Color::Reset);
        assert_eq!(parse_color("cyan").unwrap(), Color::Cyan);
        assert_eq!(
            parse_color("#336699").unwrap(),
            Color::Rgb(0x33, 0x66, 0x99)
        );
        assert_eq!(parse_color("244").unwrap(), Color::Indexed(244));
        assert!(parse_color("chartreuse-ish").is_err());
    }

    #[test]
    fn selection_bg_without_fg_is_rejected() {
        let err = parse(
            "[interaction]\nselection = { bg = \"#ff0000\" }\n",
            Mode::Dark,
        )
        .unwrap_err();
        assert!(err.to_string().contains("selection"), "{err:#}");
    }

    #[test]
    fn multi_char_glyphs_are_rejected() {
        let err = parse(
            "[charts]\nbar = { glyph = \"▓▓\", color = \"cyan\" }\n",
            Mode::Dark,
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("exactly one character"),
            "{err:#}"
        );
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(parse("[accents]\nprimry = \"cyan\"\n", Mode::Dark).is_err());
        // The pre-restructure grab-bag section must error, not silently no-op.
        assert!(parse("[colors]\nprimary = \"cyan\"\n", Mode::Dark).is_err());
    }

    #[test]
    fn ensure_bundled_writes_missing_but_never_overwrites() {
        let dir = tempdir().unwrap();
        let themes = dir.path().join("themes");

        ensure_bundled(&themes).unwrap();
        for (name, text) in BUNDLED {
            let on_disk = fs::read_to_string(themes.join(format!("{name}.toml"))).unwrap();
            assert_eq!(on_disk, text);
        }

        // A user-edited file survives the next materialization untouched.
        let edited = themes.join("journal.toml");
        fs::write(&edited, "[chrome]\nstyle = \"bordered\"\n").unwrap();
        ensure_bundled(&themes).unwrap();
        assert_eq!(
            fs::read_to_string(&edited).unwrap(),
            "[chrome]\nstyle = \"bordered\"\n"
        );
    }

    #[test]
    fn load_falls_back_to_builtin_on_a_broken_theme() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let themes = themes_dir(&config_path);
        fs::create_dir_all(&themes).unwrap();
        fs::write(themes.join("broken.toml"), "surfaces = 12\n").unwrap();

        let theme = load(&config_path, "broken", Mode::Dark);
        assert_eq!(theme, builtin(DEFAULT_THEME, Mode::Dark).unwrap());

        let missing = load(&config_path, "does-not-exist", Mode::Dark);
        assert_eq!(missing, builtin(DEFAULT_THEME, Mode::Dark).unwrap());
    }

    #[test]
    fn test_theme_override_pins_this_thread() {
        let journal = builtin("journal", Mode::Dark).unwrap();
        set_test_theme(journal);
        assert_eq!(theme(), journal);
    }
}
