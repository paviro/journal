//! The UI's semantic style seam. Widgets ask the theme for *meaning*
//! (`heading`, `positive`, `accent`, …) and get back a ratatui [`Style`], never
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
const BUNDLED: [(&str, &str); 6] = [
    ("journal", include_str!("themes/journal.toml")),
    ("classic", include_str!("themes/classic.toml")),
    ("e-ink", include_str!("themes/e-ink.toml")),
    ("blossom", include_str!("themes/blossom.toml")),
    ("fjord", include_str!("themes/fjord.toml")),
    ("grove", include_str!("themes/grove.toml")),
];

/// The theme `load` falls back to when the configured one is missing or broken.
pub(crate) const DEFAULT_THEME: &str = "journal";

/// Which variant of a `{ dark, light }` color a load resolves to. Detected from
/// the terminal background once at startup and cached for the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Dark,
    #[allow(dead_code)] // constructed once terminal detection lands
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

/// A fully resolved theme: plain styles and colors, no variants left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Theme {
    bg: Color,
    panel: Color,
    element: Color,
    text: Style,
    muted: Style,
    primary: Style,
    secondary: Style,
    accent: Style,
    border: Style,
    border_subtle: Style,
    border_active: Style,
    success: Style,
    warning: Style,
    error: Style,
    info: Style,
    selection: Style,
    key_hint: Style,
    chart_positive: Fill,
    chart_neutral: Fill,
    chart_negative: Fill,
    bar: Fill,
    track: Fill,
    chart_axis: Style,
    chart_baseline: Style,
    chart_label: Style,
    md_heading: Style,
    md_link: Style,
    md_code: Style,
    md_blockquote: Style,
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

/// The current theme. Cheap to call everywhere: `Theme` is `Copy`.
pub(crate) fn theme() -> Theme {
    #[cfg(test)]
    if let Some(theme) = TEST_THEME.with(std::cell::Cell::get) {
        return theme;
    }
    THEME
        .read()
        .expect("theme lock")
        .unwrap_or_else(Theme::terminal_default)
}

/// Swap the active theme; the next frame repaints with it. Used at startup,
/// by live reload, and by the theme picker's preview.
#[allow(dead_code)] // wired up when startup loading lands
pub(crate) fn install(theme: Theme) {
    *THEME.write().expect("theme lock") = Some(theme);
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
    #[allow(dead_code)] // consumed by the flat-chrome pass
    pub(crate) fn bg(self) -> Color {
        self.bg
    }

    /// Elevated surfaces: panels, dialogs, notices, toasts.
    #[allow(dead_code)] // consumed by the flat-chrome pass
    pub(crate) fn panel_bg(self) -> Color {
        self.panel
    }

    /// Interactive surfaces sitting on a panel: inputs, active controls.
    #[allow(dead_code)] // consumed by the flat-chrome pass
    pub(crate) fn element_bg(self) -> Color {
        self.element
    }

    // --- text ---

    /// Primary body text.
    pub(crate) fn text(self) -> Style {
        self.text
    }

    /// Section titles and emphasised labels.
    pub(crate) fn heading(self) -> Style {
        self.text.add_modifier(Modifier::BOLD)
    }

    /// Secondary text: captions, units, "+k more", empty hints.
    pub(crate) fn muted(self) -> Style {
        self.muted.add_modifier(Modifier::DIM)
    }

    // --- accents ---

    /// The app's single accent — count bars, tags/feelings fills, series
    /// glyphs. Colour is decoration here: bars already encode magnitude by
    /// length.
    #[allow(dead_code)] // consumed by the flat-chrome pass
    pub(crate) fn accent(self) -> Style {
        self.accent
    }

    /// The secondary accent hue, for places that need contrast *with* the
    /// primary accent.
    #[allow(dead_code)] // consumed by the flat-chrome pass
    pub(crate) fn secondary(self) -> Style {
        self.secondary
    }

    /// The primary accent as a style: focused titles, current-item markers.
    #[allow(dead_code)] // consumed by the flat-chrome pass
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

    /// A primary action button chip.
    #[allow(dead_code)] // consumed by the flat-chrome pass
    pub(crate) fn button(self) -> Style {
        self.selection
    }

    /// A keybinding chip/hint in the footer and dialogs.
    pub(crate) fn key_hint(self) -> Style {
        self.key_hint
    }

    /// The active tab in the tab strip: selection-styled while the panel is
    /// focused so it reads even without colour, otherwise just bold.
    pub(crate) fn active_tab(self, focused: bool) -> Style {
        if focused {
            self.selection.add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
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

    // --- charts ---

    /// Alias of the count/frequency bar fill's style.
    pub(crate) fn bar_fill(self) -> Style {
        self.bar.style
    }

    /// The filled part of count/frequency bars.
    #[allow(dead_code)] // consumed by the themed-charts pass
    pub(crate) fn chart_bar(self) -> Fill {
        self.bar
    }

    /// The empty remainder of a bar.
    #[allow(dead_code)] // consumed by the themed-charts pass
    pub(crate) fn chart_track(self) -> Fill {
        self.track
    }

    /// The positive sentiment series.
    #[allow(dead_code)] // consumed by the themed-charts pass
    pub(crate) fn chart_positive(self) -> Fill {
        self.chart_positive
    }

    /// The neutral sentiment series.
    #[allow(dead_code)] // consumed by the themed-charts pass
    pub(crate) fn chart_neutral(self) -> Fill {
        self.chart_neutral
    }

    /// The negative sentiment series.
    #[allow(dead_code)] // consumed by the themed-charts pass
    pub(crate) fn chart_negative(self) -> Fill {
        self.chart_negative
    }

    /// Chart axis ticks and edges.
    #[allow(dead_code)] // consumed by the themed-charts pass
    pub(crate) fn chart_axis(self) -> Style {
        self.chart_axis
    }

    /// The zero baseline of signed column charts.
    #[allow(dead_code)] // consumed by the themed-charts pass
    pub(crate) fn chart_baseline(self) -> Style {
        self.chart_baseline
    }

    /// Chart captions and column labels.
    #[allow(dead_code)] // consumed by the themed-charts pass
    pub(crate) fn chart_label(self) -> Style {
        self.chart_label
    }

    // --- markdown ---

    /// Markdown headings in the entry viewer.
    #[allow(dead_code)] // consumed by the themed-markdown pass
    pub(crate) fn md_heading(self) -> Style {
        self.md_heading
    }

    /// Markdown links.
    pub(crate) fn md_link(self) -> Style {
        self.md_link
    }

    /// Inline code and code blocks.
    #[allow(dead_code)] // consumed by the themed-markdown pass
    pub(crate) fn md_code(self) -> Style {
        self.md_code
    }

    /// Block quotes.
    #[allow(dead_code)] // consumed by the themed-markdown pass
    pub(crate) fn md_blockquote(self) -> Style {
        self.md_blockquote
    }

    // --- chrome ---

    /// Whether this theme separates surfaces by background or drawn borders.
    #[allow(dead_code)] // consumed by the flat-chrome pass
    pub(crate) fn chrome(self) -> ChromeStyle {
        self.chrome
    }

    /// How strongly the screen dims behind dialogs, `0.0..=1.0`. Zero means
    /// the DIM-modifier fallback.
    #[allow(dead_code)] // consumed by the scrim pass
    pub(crate) fn scrim_strength(self) -> f32 {
        self.scrim
    }

    /// Every style the theme carries, for whole-theme assertions in tests.
    #[cfg(test)]
    fn all_styles(&self) -> Vec<(&'static str, Style)> {
        vec![
            ("text", self.text),
            ("muted", self.muted),
            ("primary", self.primary),
            ("secondary", self.secondary),
            ("accent", self.accent),
            ("border", self.border),
            ("border_subtle", self.border_subtle),
            ("border_active", self.border_active),
            ("success", self.success),
            ("warning", self.warning),
            ("error", self.error),
            ("info", self.info),
            ("selection", self.selection),
            ("key_hint", self.key_hint),
            ("chart_positive", self.chart_positive.style),
            ("chart_neutral", self.chart_neutral.style),
            ("chart_negative", self.chart_negative.style),
            ("bar", self.bar.style),
            ("track", self.track.style),
            ("chart_axis", self.chart_axis),
            ("chart_baseline", self.chart_baseline),
            ("chart_label", self.chart_label),
            ("md_heading", self.md_heading),
            ("md_link", self.md_link),
            ("md_code", self.md_code),
            ("md_blockquote", self.md_blockquote),
        ]
    }
}

// --- loading ---

/// The directory holding the user-editable theme files, next to `config.toml`.
#[allow(dead_code)] // wired up when startup loading lands
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
        let mut chars = self.glyph.chars();
        let (Some(glyph), None) = (chars.next(), chars.next()) else {
            bail!(
                "glyph for `{token}` must be exactly one character, got {:?}",
                self.glyph
            );
        };
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
    colors: ColorsSection,
    charts: ChartsSection,
    markdown: MarkdownSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChromeSection {
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ColorsSection {
    bg: Option<ColorSpec>,
    panel: Option<ColorSpec>,
    element: Option<ColorSpec>,
    text: Option<TokenSpec>,
    muted: Option<TokenSpec>,
    primary: Option<TokenSpec>,
    secondary: Option<TokenSpec>,
    accent: Option<TokenSpec>,
    border: Option<TokenSpec>,
    border_subtle: Option<TokenSpec>,
    border_active: Option<TokenSpec>,
    success: Option<TokenSpec>,
    warning: Option<TokenSpec>,
    error: Option<TokenSpec>,
    info: Option<TokenSpec>,
    selection: Option<StyleSpec>,
    key_hint: Option<StyleSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChartsSection {
    positive: Option<FillSpec>,
    neutral: Option<FillSpec>,
    negative: Option<FillSpec>,
    bar: Option<FillSpec>,
    track: Option<FillSpec>,
    axis: Option<TokenSpec>,
    baseline: Option<TokenSpec>,
    label: Option<TokenSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MarkdownSection {
    heading: Option<TokenSpec>,
    link: Option<TokenSpec>,
    code: Option<TokenSpec>,
    blockquote: Option<TokenSpec>,
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

        let colors = &self.colors;
        let bg = color(&colors.bg, Color::Reset, "colors.bg")?;
        let panel = color(&colors.panel, bg, "colors.panel")?;
        let element = color(&colors.element, panel, "colors.element")?;
        let text = style(&colors.text, Style::default(), "colors.text")?;
        let muted = style(&colors.muted, Style::default(), "colors.muted")?;
        let primary = style(
            &colors.primary,
            Style::default().fg(Color::Cyan),
            "colors.primary",
        )?;
        let secondary = style(&colors.secondary, primary, "colors.secondary")?;
        let accent = style(&colors.accent, primary, "colors.accent")?;
        let border = style(
            &colors.border,
            Style::default().fg(Color::Indexed(244)),
            "colors.border",
        )?;
        let border_subtle = style(
            &colors.border_subtle,
            Style::default().fg(Color::Indexed(240)),
            "colors.border_subtle",
        )?;
        let border_active = style(
            &colors.border_active,
            Style::default(),
            "colors.border_active",
        )?;

        let selection = match &colors.selection {
            Some(spec) => {
                if spec.bg.is_some() && spec.fg.is_none() {
                    bail!(
                        "`colors.selection` sets a bg without an fg; pick a readable \
                         foreground explicitly"
                    );
                }
                spec.resolve(mode, palette, "colors.selection")?
            }
            None => Style::default().add_modifier(Modifier::REVERSED),
        };
        let key_hint = match &colors.key_hint {
            Some(spec) => spec.resolve(mode, palette, "colors.key_hint")?,
            None => Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
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
        // Axis furniture defaults to the muted ink so charts read as they
        // always have when a theme doesn't restyle them.
        let chart_furniture = muted.add_modifier(Modifier::DIM);
        let chart_axis = style(&charts.axis, chart_furniture, "charts.axis")?;
        let chart_baseline = style(&charts.baseline, chart_furniture, "charts.baseline")?;
        let chart_label = style(&charts.label, chart_furniture, "charts.label")?;

        let markdown = &self.markdown;
        let md_heading = style(&markdown.heading, Style::default(), "markdown.heading")?;
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

        Ok(Theme {
            bg,
            panel,
            element,
            text,
            muted,
            primary,
            secondary,
            accent,
            border,
            border_subtle,
            border_active,
            success: style(
                &colors.success,
                Style::default().fg(Color::Green),
                "colors.success",
            )?,
            warning: style(
                &colors.warning,
                Style::default().fg(Color::Yellow),
                "colors.warning",
            )?,
            error: style(
                &colors.error,
                Style::default().fg(Color::Red),
                "colors.error",
            )?,
            info: style(
                &colors.info,
                Style::default().fg(Color::Blue),
                "colors.info",
            )?,
            selection,
            key_hint,
            chart_positive,
            chart_neutral,
            chart_negative,
            bar,
            track,
            chart_axis,
            chart_baseline,
            chart_label,
            md_heading,
            md_link,
            md_code,
            md_blockquote,
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
        assert_eq!(theme.accent(), Style::default().fg(Color::Cyan));
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

            [colors]
            primary = "splash"
            secondary = "flat"
            "##,
            Mode::Light,
        )
        .unwrap();
        assert_eq!(theme.primary.fg, Some(Color::Rgb(0xe0, 0xe0, 0xe0)));
        assert_eq!(theme.secondary.fg, Some(Color::Rgb(0x44, 0x55, 0x66)));
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
        let err = parse("[colors]\nselection = { bg = \"#ff0000\" }\n", Mode::Dark).unwrap_err();
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
        assert!(parse("[colors]\nprimry = \"cyan\"\n", Mode::Dark).is_err());
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
        fs::write(themes.join("broken.toml"), "colors = 12\n").unwrap();

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
