//! The theme file's TOML schema and its resolution into a [`Theme`]: serde
//! section structs, palette and color parsing, and the per-token defaults and
//! inheritance applied by [`ThemeFile::resolve`].

use anyhow::{Context, Result, anyhow, bail};
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::{collections::BTreeMap, str::FromStr};

use super::{
    AlertGlyphs, BorderGlyphs, ChartRamps, ChromeStyle, CustomBorderSet, EnvGlyphs, Fill, Glyphs,
    MarkdownGlyphs, MetadataTheme, Mode, MoonGlyphs, PillStyle, Syntax, Theme, WeatherGlyphs,
    intern_chart_ramps, intern_markdown_glyphs, intern_metadata_theme,
};

pub(super) fn parse(text: &str, mode: Mode) -> Result<Theme> {
    let file: ThemeFile = toml::from_str(text).context("parsing theme TOML")?;
    if file.schema_version != 1 {
        bail!(
            "unsupported theme schema version {}; expected 1",
            file.schema_version
        );
    }
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
        let mut name = self.pick(mode);
        // Follow palette references transitively so an accent alias that points
        // at another [palette] entry (e.g. `tertiary` → a named hue) resolves
        // the same as naming that entry directly. A bounded hop count — one per
        // entry — keeps a self-referential palette from looping forever; a cycle
        // just falls through to `parse_color` and errors.
        for _ in 0..=palette.len() {
            match palette.get(name) {
                Some(entry) => name = entry.pick(mode),
                None => break,
            }
        }
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

pub(super) fn parse_color(name: &str) -> Result<Color> {
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ThemeFile {
    schema_version: u32,
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
    toast: ToastSection,
    tabs: TabsSection,
    metadata: MetadataSection,
    indicators: IndicatorsSection,
}

/// Small stateful UI markers that carry no color of their own (they ride the
/// surrounding text style). Glyphs only, like [`ToastSection`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct IndicatorsSection {
    glyphs: IndicatorsGlyphsSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct IndicatorsGlyphsSection {
    /// The disclosure marker for an expanded group.
    expanded: Option<String>,
    /// The disclosure marker for a collapsed group.
    collapsed: Option<String>,
    /// The marker trailing a starred entry.
    starred: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChromeSection {
    /// The theme's preferred chrome — a preference, not a mandate, because the
    /// `[ui] chrome` setting can force flat/bordered on any theme.
    default_style: ChromeStyle,
    scrim: f32,
}

impl Default for ChromeSection {
    fn default() -> Self {
        Self {
            default_style: ChromeStyle::Bordered,
            scrim: 0.0,
        }
    }
}

/// The surface layers the UI is built from, base to top.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SurfacesSection {
    base: Option<ColorSpec>,
    content: Option<ColorSpec>,
    dialog: Option<ColorSpec>,
    raised: Option<ColorSpec>,
    footer: Option<ColorSpec>,
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
    secondary: Option<TokenSpec>,
    tertiary: Option<TokenSpec>,
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
    /// Per-glyph overrides on `style` — a theme's own character set. Omitted
    /// keys inherit the base style's glyph.
    glyphs: Option<BorderGlyphsSection>,
    /// The set focused panels switch to, replacing the default thick promotion.
    focused_style: Option<BorderGlyphs>,
    /// Per-glyph overrides for the focused set, layered on `focused_style` (or
    /// the base set when no `focused_style` is given).
    focused_glyphs: Option<BorderGlyphsSection>,
    subtle: Option<TokenSpec>,
    focused: Option<TokenSpec>,
    unfocused: Option<TokenSpec>,
    /// The rule of section dividers (month headers, "Archived"). Defaults to the
    /// muted ink the divider has always used.
    divider: Option<TokenSpec>,
    /// The outline of entry/journal/stat cards. Defaults to ANSI 244 — the
    /// quiet grey the card border has always used.
    card: Option<TokenSpec>,
}

/// A custom border character set: corners plus edges, alongside the two
/// standalone furniture glyphs that live with the border look. Junction
/// characters (tees, cross) always inherit from the base style — tables need
/// them, but six keys cover what a box is made of.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct BorderGlyphsSection {
    top_left: Option<String>,
    top_right: Option<String>,
    bottom_left: Option<String>,
    bottom_right: Option<String>,
    horizontal: Option<String>,
    vertical: Option<String>,
    /// The stripe down a focused panel's left edge — the flat-chrome stand-in
    /// for the focused border, drawn in the `focused` color. Read directly by
    /// `resolve`, not part of the box-set overlay below.
    focus_stripe: Option<String>,
    /// The rule of section dividers (month headers, "Archived"). Read directly
    /// by `resolve`, not part of the box-set overlay.
    divider: Option<String>,
    /// The plain full-width rule separating dialog sections. Furniture, read
    /// directly by `resolve`.
    separator: Option<String>,
}

/// ratatui border sets hold `&'static str`, so parsed glyphs are interned
/// once per distinct character — the picker and live reload re-parse themes
/// constantly, and leaking per parse would grow without bound.
fn intern_glyph(glyph: char) -> &'static str {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<BTreeMap<char, &'static str>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .expect("glyph intern lock");
    cache
        .entry(glyph)
        .or_insert_with(|| glyph.to_string().leak())
}

impl BorderGlyphsSection {
    /// Whether any box-drawing glyph is overridden. `focus_stripe`/`divider` are
    /// furniture, not box glyphs — a section with only those keeps the base
    /// style (so its thick focus-promotion survives) instead of collapsing to a
    /// custom set that has no thick variant.
    fn has_box_overrides(&self) -> bool {
        self.top_left.is_some()
            || self.top_right.is_some()
            || self.bottom_left.is_some()
            || self.bottom_right.is_some()
            || self.horizontal.is_some()
            || self.vertical.is_some()
    }

    /// Overlay this section's glyphs on `base`, producing a custom set.
    fn resolve(&self, base: BorderGlyphs, token: &str) -> Result<BorderGlyphs> {
        let mut border = base.border_set();
        let mut line = base.line_set();
        let glyph = |spec: &Option<String>, key: &str| -> Result<Option<&'static str>> {
            spec.as_deref()
                .map(|spec| parse_glyph(spec, &format!("{token}.{key}")).map(intern_glyph))
                .transpose()
        };
        if let Some(g) = glyph(&self.top_left, "top_left")? {
            border.top_left = g;
            line.top_left = g;
        }
        if let Some(g) = glyph(&self.top_right, "top_right")? {
            border.top_right = g;
            line.top_right = g;
        }
        if let Some(g) = glyph(&self.bottom_left, "bottom_left")? {
            border.bottom_left = g;
            line.bottom_left = g;
        }
        if let Some(g) = glyph(&self.bottom_right, "bottom_right")? {
            border.bottom_right = g;
            line.bottom_right = g;
        }
        if let Some(g) = glyph(&self.horizontal, "horizontal")? {
            border.horizontal_top = g;
            border.horizontal_bottom = g;
            line.horizontal = g;
        }
        if let Some(g) = glyph(&self.vertical, "vertical")? {
            border.vertical_left = g;
            border.vertical_right = g;
            line.vertical = g;
        }
        Ok(BorderGlyphs::Custom(intern_border_set(CustomBorderSet {
            border,
            line,
        })))
    }
}

/// Intern a resolved border set — leaked once per distinct set so
/// [`BorderGlyphs`] can carry a `Copy` reference.
fn intern_border_set(set: CustomBorderSet) -> &'static CustomBorderSet {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Vec<&'static CustomBorderSet>>> = OnceLock::new();
    super::intern(set, &CACHE)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct InteractionSection {
    selection: Option<StyleSpec>,
    hover: Option<StyleSpec>,
    button: Option<StyleSpec>,
    /// The style layered onto a button chip under the mouse. Defaults to an
    /// underline, patched over the button style, so a theme can pick a different
    /// hover treatment (bold, a bg lift) without losing the chip's own colors.
    button_hover: Option<StyleSpec>,
    key_hint: Option<StyleSpec>,
    cursor: Option<StyleSpec>,
    cursor_line: Option<StyleSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ScrollbarSection {
    thumb: Option<TokenSpec>,
    track: Option<TokenSpec>,
    /// The up/down arrow caps. Defaults to the thumb hue so they read as part
    /// of the handle.
    arrow: Option<TokenSpec>,
    glyphs: ScrollbarGlyphsSection,
}

/// The scrollbar's characters, defaulting to ratatui's own vertical set.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ScrollbarGlyphsSection {
    thumb: Option<String>,
    track: Option<String>,
    up: Option<String>,
    down: Option<String>,
}

/// Chart *colors* only — glyphs live in the parallel `[charts.glyphs]` section,
/// matching every other themable section (`[scrollbar]`/`[scrollbar.glyphs]`,
/// `[borders]`/`[borders.glyphs]`, …). The meaning-carrying modifiers (bold on
/// signed series, dim on neutral/track) are added in code.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChartsSection {
    positive: Option<TokenSpec>,
    neutral: Option<TokenSpec>,
    negative: Option<TokenSpec>,
    bar: Option<TokenSpec>,
    track: Option<TokenSpec>,
    baseline: Option<TokenSpec>,
    label: Option<TokenSpec>,
    glyphs: ChartsGlyphsSection,
}

/// Every glyph a chart draws. `ramp_up`/`ramp_down` are the eighths ramps for
/// vertical bars and are the only multi-character keys; all others are exactly
/// one character.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChartsGlyphsSection {
    positive: Option<String>,
    neutral: Option<String>,
    negative: Option<String>,
    bar: Option<String>,
    track: Option<String>,
    diverge_track: Option<String>,
    diverge_center: Option<String>,
    baseline: Option<String>,
    rule: Option<String>,
    ramp_up: Option<String>,
    ramp_down: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MarkdownSection {
    heading: Option<TokenSpec>,
    heading2: Option<TokenSpec>,
    subheading: Option<TokenSpec>,
    link: Option<TokenSpec>,
    code: Option<TokenSpec>,
    /// Inline `` `code` `` spans. Defaults to `code` so both read alike until a
    /// theme splits them (e.g. an inline chip with a background).
    inline_code: Option<TokenSpec>,
    blockquote: Option<TokenSpec>,
    /// `==highlight==` spans. Defaults to the primary accent, reversed + bold.
    highlight: Option<TokenSpec>,
    syntax: SyntaxSection,
    glyphs: MarkdownGlyphsSection,
}

/// The markdown reader's structural chrome. Multi-character values (a rail is
/// `│ `, a fence corner `╭─`), so plain strings — not single glyphs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MarkdownGlyphsSection {
    quote_rail: Option<String>,
    code_rail: Option<String>,
    code_top: Option<String>,
    code_bottom: Option<String>,
    /// The unordered-list bullet (`-` by default). One character; ordered lists
    /// keep their `N.` numbering.
    bullet: Option<String>,
    /// The task-list checkboxes, done and to-do (`[x]` / `[ ]` by default). Short
    /// strings, so a theme can use single-glyph boxes (`☑` / `☐`) instead.
    task_done: Option<String>,
    task_todo: Option<String>,
    alert: AlertGlyphsSection,
}

/// The icon leading each GitHub-style alert blockquote. One character each; the
/// band colors ride the status hues (`info`/`success`/`primary`/`warning`/`error`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AlertGlyphsSection {
    note: Option<String>,
    tip: Option<String>,
    important: Option<String>,
    warning: Option<String>,
    caution: Option<String>,
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ToastSection {
    glyphs: ToastGlyphsSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ToastGlyphsSection {
    /// The accent edges of a toast card (flat chrome).
    edge: Option<String>,
    /// The dismissal countdown line along a toast's bottom edge.
    progress: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TabsSection {
    /// The separator glyph's ink between tab labels. Defaults to the muted ink.
    separator: Option<TokenSpec>,
    glyphs: TabsGlyphsSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TabsGlyphsSection {
    /// The separator between tab labels; always rendered with a space each
    /// side so the strip's width math stays fixed.
    separator: Option<String>,
}

/// The reader's entry-metadata section: pill chips, environment-strip accents,
/// and the strip's glyph vocabulary.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MetadataSection {
    pills: PillsSection,
    environment: EnvironmentSection,
    glyphs: MetadataGlyphsSection,
}

/// How feelings/people/activities/tags chips are drawn. Colors only apply to
/// the `bg` style; `reversed` stays code-enforced (monochrome contract) and
/// `bracket` is plain text.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PillsSection {
    style: PillStyle,
    feelings: Option<StyleSpec>,
    people: Option<StyleSpec>,
    activities: Option<StyleSpec>,
    tags: Option<StyleSpec>,
}

/// The environment strip's air-quality bands. Defaults ride the status hues so
/// a bad-air badge reads as the warning/error it is.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EnvironmentSection {
    aqi_poor: Option<TokenSpec>,
    aqi_very_poor: Option<TokenSpec>,
    aqi_extremely_poor: Option<TokenSpec>,
    /// The strip's high-pollen badge. Defaults to the warning hue — like the
    /// first AQI band, it only appears when it is a warning.
    pollen_high: Option<TokenSpec>,
    /// The mood gauge's filled cells, by valence. Default to the status hues so
    /// a low mood reads red, a high mood green, on any theme.
    mood_negative: Option<TokenSpec>,
    mood_positive: Option<TokenSpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MetadataGlyphsSection {
    /// The full-width rule above the metadata block (both layout paths).
    rule: Option<String>,
    /// The dot between environment-strip items; always rendered with a space
    /// each side so the strip's width math stays fixed.
    separator: Option<String>,
    location: Option<String>,
    sunrise: Option<String>,
    sunset: Option<String>,
    /// The dot leading the air-quality badge.
    aqi: Option<String>,
    /// The marker leading the high-pollen badge.
    pollen: Option<String>,
    /// The mood bar's filled and empty cells (the center marker is the shared
    /// `charts.glyphs.diverge_center`).
    mood_fill: Option<String>,
    mood_track: Option<String>,
    /// The glyph leading each chip pill, by category — echoing the strip's
    /// glyph-led grammar so the pill row reads as one family with it.
    feelings: Option<String>,
    people: Option<String>,
    activities: Option<String>,
    tags: Option<String>,
    weather: WeatherGlyphsSection,
    moon: MoonGlyphsSection,
}

/// One key per weather-condition slug the context provider emits.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WeatherGlyphsSection {
    clear: Option<String>,
    mostly_clear: Option<String>,
    partly_cloudy: Option<String>,
    cloudy: Option<String>,
    fog: Option<String>,
    drizzle: Option<String>,
    rain: Option<String>,
    snow: Option<String>,
    thunderstorm: Option<String>,
}

/// One key per moon-phase slug the celestial provider emits.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MoonGlyphsSection {
    new: Option<String>,
    waxing_crescent: Option<String>,
    first_quarter: Option<String>,
    waxing_gibbous: Option<String>,
    full: Option<String>,
    waning_gibbous: Option<String>,
    last_quarter: Option<String>,
    waning_crescent: Option<String>,
}

/// A single-character glyph value.
fn parse_glyph(spec: &str, token: &str) -> Result<char> {
    let mut chars = spec.chars();
    let (Some(glyph), None) = (chars.next(), chars.next()) else {
        bail!("glyph for `{token}` must be exactly one character, got {spec:?}");
    };
    Ok(glyph)
}

/// A multi-character glyph string (a rail like `│ `, a fence corner like `╭─`),
/// falling back to `default` when unset. Must be non-empty and single-line: an
/// empty or multi-line value breaks the per-line reader chrome.
fn string_glyph(spec: &Option<String>, default: &str, token: &str) -> Result<String> {
    let value = spec.clone().unwrap_or_else(|| default.to_string());
    if value.is_empty() || value.contains('\n') {
        bail!("markdown glyph for `{token}` must be non-empty and single-line, got {value:?}");
    }
    Ok(value)
}

/// An eighths ramp: exactly `N` glyphs, darkest-empty first.
fn parse_ramp<const N: usize>(spec: &str, token: &str) -> Result<[char; N]> {
    let ramp: Vec<char> = spec.chars().collect();
    let ramp: [char; N] = ramp.try_into().map_err(|got: Vec<char>| {
        anyhow!(
            "ramp for `{token}` must be exactly {N} characters, got {}",
            got.len()
        )
    })?;
    Ok(ramp)
}

impl ThemeFile {
    /// Flatten the file into a [`Theme`] for one [`Mode`]. Omitted tokens fall
    /// back to the classic look, so an empty file *is* `classic.toml`.
    pub(super) fn resolve(&self, mode: Mode) -> Result<Theme> {
        // Seed the palette with the three accents so any color token can name
        // them (`fg = "secondary"`). A theme's own [palette] entry of the same
        // name wins; the defaults chain secondary → primary → cyan and
        // tertiary → secondary, so a theme that sets only `primary` still gets
        // coherent hues for free.
        let accent_colorspec = |spec: &Option<TokenSpec>| -> Option<ColorSpec> {
            match spec {
                Some(TokenSpec::Color(cs)) => Some(cs.clone()),
                Some(TokenSpec::Style(ss)) => ss.fg.clone(),
                None => None,
            }
        };
        let primary_cs = accent_colorspec(&self.accents.primary)
            .unwrap_or_else(|| ColorSpec::Single("cyan".into()));
        let secondary_cs =
            accent_colorspec(&self.accents.secondary).unwrap_or_else(|| primary_cs.clone());
        let tertiary_cs =
            accent_colorspec(&self.accents.tertiary).unwrap_or_else(|| secondary_cs.clone());
        let mut seeded = self.palette.clone();
        seeded.entry("primary".into()).or_insert(primary_cs);
        seeded.entry("secondary".into()).or_insert(secondary_cs);
        seeded.entry("tertiary".into()).or_insert(tertiary_cs);
        let palette = &seeded;
        let color = |spec: &Option<ColorSpec>, default: Color, token: &str| -> Result<Color> {
            spec.as_ref()
                .map_or(Ok(default), |spec| spec.resolve(mode, palette, token))
        };
        let style = |spec: &Option<TokenSpec>, default: Style, token: &str| -> Result<Style> {
            spec.as_ref()
                .map_or(Ok(default), |spec| spec.resolve(mode, palette, token))
        };
        // A chart fill draws its color from `[charts]` and its glyph from
        // `[charts.glyphs]`; the meaning-carrying modifier always comes from code.
        let fill = |color: &Option<TokenSpec>,
                    glyph_spec: &Option<String>,
                    default_glyph: char,
                    default_style: Style,
                    carries: Modifier,
                    color_token: &str,
                    glyph_token: &str|
         -> Result<Fill> {
            let base = match color {
                Some(spec) => spec.resolve(mode, palette, color_token)?,
                None => default_style,
            };
            let glyph = match glyph_spec {
                Some(spec) => parse_glyph(spec, glyph_token)?,
                None => default_glyph,
            };
            Ok(Fill {
                glyph,
                style: base.add_modifier(carries),
            })
        };

        let surfaces = &self.surfaces;
        let base = color(&surfaces.base, Color::Reset, "surfaces.base")?;
        let content = color(&surfaces.content, base, "surfaces.content")?;
        let dialog = color(&surfaces.dialog, content, "surfaces.dialog")?;
        let raised = color(&surfaces.raised, content, "surfaces.raised")?;
        let footer = color(&surfaces.footer, base, "surfaces.footer")?;
        let text = style(&self.text.body, Style::default(), "text.body")?;
        let muted = style(&self.text.muted, Style::default(), "text.muted")?;
        let heading = style(&self.text.heading, text, "text.heading")?;
        let placeholder = style(&self.text.placeholder, muted, "text.placeholder")?;
        let primary = style(
            &self.accents.primary,
            Style::default().fg(Color::Cyan),
            "accents.primary",
        )?;
        let secondary = style(&self.accents.secondary, primary, "accents.secondary")?;
        let borders = &self.borders;
        let border_subtle = style(
            &borders.subtle,
            Style::default().fg(Color::Indexed(240)),
            "borders.subtle",
        )?;
        let border_active = style(&borders.focused, Style::default(), "borders.focused")?;
        let border_inactive = style(&borders.unfocused, Style::default(), "borders.unfocused")?;
        // Structural furniture: the divider rule and the tab separator have
        // always ridden the muted ink (dim included, as `Theme::muted` applies);
        // cards have used the normal border. Each is now a token that keeps that
        // default.
        let muted_ink = muted.add_modifier(Modifier::DIM);
        let divider = style(&borders.divider, muted_ink, "borders.divider")?;
        let card_border = style(
            &borders.card,
            Style::default().fg(Color::Indexed(244)),
            "borders.card",
        )?;
        let tab_separator = style(&self.tabs.separator, muted_ink, "tabs.separator")?;
        // The thumb inherits the focused-border hue (it marks the scrollable,
        // interactable panel); the track stays terminal-default quiet.
        let scrollbar_thumb = style(&self.scrollbar.thumb, border_active, "scrollbar.thumb")?;
        let scrollbar_track = style(&self.scrollbar.track, Style::default(), "scrollbar.track")?;
        let scrollbar_arrow = style(&self.scrollbar.arrow, scrollbar_thumb, "scrollbar.arrow")?;

        let interaction = &self.interaction;
        // Selection and buttons fill their whole row/chip, so a bg that
        // replaces the row's ink must bring a readable fg with it.
        let readable_fill = |spec: &Option<StyleSpec>, fallback: Style, token: &str| match spec {
            Some(spec) => {
                if spec.bg.is_some() && spec.fg.is_none() {
                    bail!(
                        "`{token}` sets a bg without an fg; pick a readable \
                             foreground explicitly"
                    );
                }
                spec.resolve(mode, palette, token)
            }
            None => Ok(fallback),
        };
        let selection = readable_fill(
            &interaction.selection,
            Style::default().add_modifier(Modifier::REVERSED),
            "interaction.selection",
        )?;
        // Unlike selection, a bg-only hover is fine: it layers under the
        // row's existing foreground instead of replacing it. The default
        // *lifts* the raised surface rather than reusing it: raised is what
        // cards/chips already sit on, so a same-color hover would be invisible
        // exactly where hover matters most — and theme files written before
        // the token existed (never overwritten on upgrade) hit this default.
        let hover = match &interaction.hover {
            Some(spec) => spec.resolve(mode, palette, "interaction.hover")?,
            None => Style::default().bg(lift(raised, mode)),
        };
        // Buttons are selection-colored unless a theme splits them.
        let button = readable_fill(&interaction.button, selection, "interaction.button")?;
        // The hover treatment is patched over the button chip, so its default is
        // a bare underline (matching the long-standing hardcoded behavior).
        let button_hover = match &interaction.button_hover {
            Some(spec) => spec.resolve(mode, palette, "interaction.button_hover")?,
            None => Style::default().add_modifier(Modifier::UNDERLINED),
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
        let chart_glyphs = &charts.glyphs;
        // The classic chart palette as fill defaults; bold/dim are the
        // monochrome contract and always come from code.
        let chart_positive = fill(
            &charts.positive,
            &chart_glyphs.positive,
            '▓',
            Style::default().fg(Color::Green),
            Modifier::BOLD,
            "charts.positive",
            "charts.glyphs.positive",
        )?;
        let chart_neutral = fill(
            &charts.neutral,
            &chart_glyphs.neutral,
            '▓',
            Style::default(),
            Modifier::DIM,
            "charts.neutral",
            "charts.glyphs.neutral",
        )?;
        let chart_negative = fill(
            &charts.negative,
            &chart_glyphs.negative,
            '▓',
            Style::default().fg(Color::Red),
            Modifier::BOLD,
            "charts.negative",
            "charts.glyphs.negative",
        )?;
        let bar = fill(
            &charts.bar,
            &chart_glyphs.bar,
            '▓',
            Style::default().fg(Color::Cyan),
            Modifier::empty(),
            "charts.bar",
            "charts.glyphs.bar",
        )?;
        let track = fill(
            &charts.track,
            &chart_glyphs.track,
            '░',
            Style::default(),
            Modifier::DIM,
            "charts.track",
            "charts.glyphs.track",
        )?;
        // Chart furniture defaults to the muted ink so charts read as they
        // always have when a theme doesn't restyle them.
        let chart_furniture = muted.add_modifier(Modifier::DIM);
        let chart_baseline = style(&charts.baseline, chart_furniture, "charts.baseline")?;
        let chart_label = style(&charts.label, chart_furniture, "charts.label")?;

        let markdown = &self.markdown;
        let md_heading = style(&markdown.heading, Style::default(), "markdown.heading")?;
        let md_heading2 = style(&markdown.heading2, md_heading, "markdown.heading2")?;
        let md_subheading = style(&markdown.subheading, md_heading, "markdown.subheading")?;
        let md_link = style(
            &markdown.link,
            Style::default().add_modifier(Modifier::UNDERLINED),
            "markdown.link",
        )?;
        let md_code = style(&markdown.code, Style::default(), "markdown.code")?;
        let md_inline_code = style(&markdown.inline_code, md_code, "markdown.inline_code")?;
        let md_blockquote = style(
            &markdown.blockquote,
            Style::default(),
            "markdown.blockquote",
        )?;
        // `==highlight==` defaults to the primary accent, reversed and bold — the
        // long-standing hardcoded look — until a theme restyles it.
        let md_highlight = style(
            &markdown.highlight,
            primary.add_modifier(Modifier::REVERSED | Modifier::BOLD),
            "markdown.highlight",
        )?;

        if !(0.0..=1.0).contains(&self.chrome.scrim) {
            bail!("`chrome.scrim` must be between 0.0 and 1.0");
        }

        let status = &self.status;
        let success = style(
            &status.success,
            Style::default().fg(Color::Green),
            "status.success",
        )?;
        let warning = style(
            &status.warning,
            Style::default().fg(Color::Yellow),
            "status.warning",
        )?;
        let error = style(
            &status.error,
            Style::default().fg(Color::Red),
            "status.error",
        )?;
        let info = style(
            &status.info,
            Style::default().fg(Color::Blue),
            "status.info",
        )?;

        let pills = &self.metadata.pills;
        // A pill bg layers under the value's own ink, so bg-only specs are
        // fine. Unset categories ride the hover style — the same subtle
        // surface lift — so a theme that only sets `style = "bg"` still gets
        // visible, coherent chips.
        let pill = |spec: &Option<StyleSpec>, token: &str| -> Result<Style> {
            match spec {
                Some(spec) => spec.resolve(mode, palette, token),
                None => Ok(hover),
            }
        };
        let pill_feelings = pill(&pills.feelings, "metadata.pills.feelings")?;
        let pill_people = pill(&pills.people, "metadata.pills.people")?;
        let pill_activities = pill(&pills.activities, "metadata.pills.activities")?;
        let pill_tags = pill(&pills.tags, "metadata.pills.tags")?;
        let environment = &self.metadata.environment;
        let aqi_poor = style(
            &environment.aqi_poor,
            warning,
            "metadata.environment.aqi_poor",
        )?;
        let aqi_very_poor = style(
            &environment.aqi_very_poor,
            error,
            "metadata.environment.aqi_very_poor",
        )?;
        let aqi_extremely_poor = style(
            &environment.aqi_extremely_poor,
            error,
            "metadata.environment.aqi_extremely_poor",
        )?;
        let pollen_high = style(
            &environment.pollen_high,
            warning,
            "metadata.environment.pollen_high",
        )?;
        let mood_negative = style(
            &environment.mood_negative,
            error,
            "metadata.environment.mood_negative",
        )?;
        let mood_positive = style(
            &environment.mood_positive,
            success,
            "metadata.environment.mood_positive",
        )?;

        let glyph = |spec: &Option<String>, default: char, token: &str| -> Result<char> {
            spec.as_deref()
                .map_or(Ok(default), |spec| parse_glyph(spec, token))
        };
        let base_borders = borders.style.unwrap_or_default();
        let border_glyphs = match &borders.glyphs {
            Some(section) if section.has_box_overrides() => {
                section.resolve(base_borders, "borders.glyphs")?
            }
            _ => base_borders,
        };
        let focused_borders = match &borders.focused_glyphs {
            Some(section) if section.has_box_overrides() => Some(section.resolve(
                borders.focused_style.unwrap_or(border_glyphs),
                "borders.focused_glyphs",
            )?),
            _ => borders.focused_style,
        };
        let scrollbar_glyphs = &self.scrollbar.glyphs;
        let metadata_glyphs = &self.metadata.glyphs;
        let weather_glyphs = &metadata_glyphs.weather;
        let moon_glyphs = &metadata_glyphs.moon;
        let weather = WeatherGlyphs {
            clear: glyph(&weather_glyphs.clear, '☀', "metadata.glyphs.weather.clear")?,
            mostly_clear: glyph(
                &weather_glyphs.mostly_clear,
                '☼',
                "metadata.glyphs.weather.mostly_clear",
            )?,
            partly_cloudy: glyph(
                &weather_glyphs.partly_cloudy,
                '☁',
                "metadata.glyphs.weather.partly_cloudy",
            )?,
            cloudy: glyph(
                &weather_glyphs.cloudy,
                '☁',
                "metadata.glyphs.weather.cloudy",
            )?,
            fog: glyph(&weather_glyphs.fog, '≡', "metadata.glyphs.weather.fog")?,
            drizzle: glyph(
                &weather_glyphs.drizzle,
                '☂',
                "metadata.glyphs.weather.drizzle",
            )?,
            rain: glyph(&weather_glyphs.rain, '☂', "metadata.glyphs.weather.rain")?,
            snow: glyph(&weather_glyphs.snow, '❄', "metadata.glyphs.weather.snow")?,
            thunderstorm: glyph(
                &weather_glyphs.thunderstorm,
                '↯',
                "metadata.glyphs.weather.thunderstorm",
            )?,
        };
        let moon = MoonGlyphs {
            new: glyph(&moon_glyphs.new, '○', "metadata.glyphs.moon.new")?,
            waxing_crescent: glyph(
                &moon_glyphs.waxing_crescent,
                '☽',
                "metadata.glyphs.moon.waxing_crescent",
            )?,
            first_quarter: glyph(
                &moon_glyphs.first_quarter,
                '◐',
                "metadata.glyphs.moon.first_quarter",
            )?,
            waxing_gibbous: glyph(
                &moon_glyphs.waxing_gibbous,
                '◐',
                "metadata.glyphs.moon.waxing_gibbous",
            )?,
            full: glyph(&moon_glyphs.full, '●', "metadata.glyphs.moon.full")?,
            waning_gibbous: glyph(
                &moon_glyphs.waning_gibbous,
                '◑',
                "metadata.glyphs.moon.waning_gibbous",
            )?,
            last_quarter: glyph(
                &moon_glyphs.last_quarter,
                '◑',
                "metadata.glyphs.moon.last_quarter",
            )?,
            waning_crescent: glyph(
                &moon_glyphs.waning_crescent,
                '☾',
                "metadata.glyphs.moon.waning_crescent",
            )?,
        };
        // `focus_stripe` and `divider` live under `[borders.glyphs]` but resolve
        // out-of-band — they are standalone furniture, not part of the box set
        // the `[borders.glyphs]` overlay assembles.
        let border_furniture = borders.glyphs.as_ref();
        let glyphs = Glyphs {
            focus_stripe: glyph(
                &border_furniture.and_then(|g| g.focus_stripe.clone()),
                '┃',
                "borders.glyphs.focus_stripe",
            )?,
            toast_edge: glyph(&self.toast.glyphs.edge, '┃', "toast.glyphs.edge")?,
            toast_progress: glyph(&self.toast.glyphs.progress, '─', "toast.glyphs.progress")?,
            tab_separator: glyph(&self.tabs.glyphs.separator, '·', "tabs.glyphs.separator")?,
            divider: glyph(
                &border_furniture.and_then(|g| g.divider.clone()),
                '━',
                "borders.glyphs.divider",
            )?,
            separator: glyph(
                &border_furniture.and_then(|g| g.separator.clone()),
                '─',
                "borders.glyphs.separator",
            )?,
            chart_baseline: glyph(&chart_glyphs.baseline, '┈', "charts.glyphs.baseline")?,
            chart_rule: glyph(&chart_glyphs.rule, '─', "charts.glyphs.rule")?,
            diverge_track: glyph(
                &chart_glyphs.diverge_track,
                '·',
                "charts.glyphs.diverge_track",
            )?,
            diverge_center: glyph(
                &chart_glyphs.diverge_center,
                '│',
                "charts.glyphs.diverge_center",
            )?,
            ramps: intern_chart_ramps(ChartRamps {
                up: match &chart_glyphs.ramp_up {
                    Some(spec) => parse_ramp(spec, "charts.glyphs.ramp_up")?,
                    None => [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'],
                },
                down: match &chart_glyphs.ramp_down {
                    Some(spec) => parse_ramp(spec, "charts.glyphs.ramp_down")?,
                    None => [' ', '▔', '▀', '█'],
                },
            }),
            scrollbar_thumb: glyph(&scrollbar_glyphs.thumb, '█', "scrollbar.glyphs.thumb")?,
            scrollbar_track: glyph(&scrollbar_glyphs.track, '║', "scrollbar.glyphs.track")?,
            scrollbar_up: glyph(&scrollbar_glyphs.up, '▲', "scrollbar.glyphs.up")?,
            scrollbar_down: glyph(&scrollbar_glyphs.down, '▼', "scrollbar.glyphs.down")?,
            expanded: glyph(
                &self.indicators.glyphs.expanded,
                '▾',
                "indicators.glyphs.expanded",
            )?,
            collapsed: glyph(
                &self.indicators.glyphs.collapsed,
                '▸',
                "indicators.glyphs.collapsed",
            )?,
            starred: glyph(
                &self.indicators.glyphs.starred,
                '★',
                "indicators.glyphs.starred",
            )?,
            markdown: intern_markdown_glyphs(MarkdownGlyphs {
                quote_rail: string_glyph(
                    &self.markdown.glyphs.quote_rail,
                    "│ ",
                    "markdown.glyphs.quote_rail",
                )?,
                code_rail: string_glyph(
                    &self.markdown.glyphs.code_rail,
                    "│ ",
                    "markdown.glyphs.code_rail",
                )?,
                code_top: string_glyph(
                    &self.markdown.glyphs.code_top,
                    "╭─",
                    "markdown.glyphs.code_top",
                )?,
                code_bottom: string_glyph(
                    &self.markdown.glyphs.code_bottom,
                    "╰─",
                    "markdown.glyphs.code_bottom",
                )?,
                bullet: glyph(&self.markdown.glyphs.bullet, '-', "markdown.glyphs.bullet")?,
                task_done: string_glyph(
                    &self.markdown.glyphs.task_done,
                    "[x]",
                    "markdown.glyphs.task_done",
                )?,
                task_todo: string_glyph(
                    &self.markdown.glyphs.task_todo,
                    "[ ]",
                    "markdown.glyphs.task_todo",
                )?,
                alert: AlertGlyphs {
                    note: glyph(
                        &self.markdown.glyphs.alert.note,
                        'i',
                        "markdown.glyphs.alert.note",
                    )?,
                    tip: glyph(
                        &self.markdown.glyphs.alert.tip,
                        '*',
                        "markdown.glyphs.alert.tip",
                    )?,
                    important: glyph(
                        &self.markdown.glyphs.alert.important,
                        '!',
                        "markdown.glyphs.alert.important",
                    )?,
                    warning: glyph(
                        &self.markdown.glyphs.alert.warning,
                        '!',
                        "markdown.glyphs.alert.warning",
                    )?,
                    caution: glyph(
                        &self.markdown.glyphs.alert.caution,
                        '!',
                        "markdown.glyphs.alert.caution",
                    )?,
                },
            }),
            borders: border_glyphs,
            focused_borders,
        };
        let metadata = intern_metadata_theme(MetadataTheme {
            pill_style: pills.style,
            pill_feelings,
            pill_people,
            pill_activities,
            pill_tags,
            aqi_poor,
            aqi_very_poor,
            aqi_extremely_poor,
            pollen_high,
            mood_negative,
            mood_positive,
            glyphs: EnvGlyphs {
                rule: glyph(&metadata_glyphs.rule, '─', "metadata.glyphs.rule")?,
                separator: glyph(&metadata_glyphs.separator, '·', "metadata.glyphs.separator")?,
                location: glyph(&metadata_glyphs.location, '⚑', "metadata.glyphs.location")?,
                sunrise: glyph(&metadata_glyphs.sunrise, '↑', "metadata.glyphs.sunrise")?,
                sunset: glyph(&metadata_glyphs.sunset, '↓', "metadata.glyphs.sunset")?,
                aqi: glyph(&metadata_glyphs.aqi, '▲', "metadata.glyphs.aqi")?,
                pollen: glyph(&metadata_glyphs.pollen, '❀', "metadata.glyphs.pollen")?,
                mood_fill: glyph(&metadata_glyphs.mood_fill, '▓', "metadata.glyphs.mood_fill")?,
                mood_track: glyph(
                    &metadata_glyphs.mood_track,
                    '░',
                    "metadata.glyphs.mood_track",
                )?,
                feelings: glyph(&metadata_glyphs.feelings, '♥', "metadata.glyphs.feelings")?,
                people: glyph(&metadata_glyphs.people, '@', "metadata.glyphs.people")?,
                activities: glyph(
                    &metadata_glyphs.activities,
                    '◆',
                    "metadata.glyphs.activities",
                )?,
                tags: glyph(&metadata_glyphs.tags, '#', "metadata.glyphs.tags")?,
                weather,
                moon,
            },
        });

        Ok(Theme {
            base,
            content,
            dialog,
            raised,
            footer,
            text,
            muted,
            heading,
            placeholder,
            primary,
            secondary,
            border_subtle,
            divider,
            card_border,
            tab_separator,
            border_active,
            border_inactive,
            success,
            warning,
            error,
            info,
            selection,
            hover,
            button,
            button_hover,
            key_hint,
            cursor,
            cursor_line,
            scrollbar_thumb,
            scrollbar_arrow,
            scrollbar_track,
            chart_positive,
            chart_neutral,
            chart_negative,
            chart_bar: bar,
            chart_track: track,
            chart_baseline,
            chart_label,
            md_heading,
            md_heading2,
            md_subheading,
            md_link,
            md_code,
            md_inline_code,
            md_blockquote,
            md_highlight,
            syntax: markdown.syntax.resolve(mode, palette)?,
            metadata,
            glyphs,
            chrome: self.chrome.default_style,
            scrim: self.chrome.scrim,
        })
    }
}
