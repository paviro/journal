//! The theme file's TOML schema and its resolution into a [`Theme`]: serde
//! section structs, palette and color parsing, and the per-token defaults and
//! inheritance applied by [`ThemeFile::resolve`].

use anyhow::{Context, Result, anyhow, bail};
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::{collections::BTreeMap, str::FromStr};

use super::{BorderGlyphs, ChromeStyle, CustomBorderSet, Fill, Glyphs, Mode, Syntax, Theme};

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
    element: Option<ColorSpec>,
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
    normal: Option<TokenSpec>,
    subtle: Option<TokenSpec>,
    focused: Option<TokenSpec>,
    unfocused: Option<TokenSpec>,
    /// The rule of section dividers (month headers, "Archived"). Defaults to the
    /// muted ink the divider has always used.
    divider: Option<TokenSpec>,
    /// The outline of entry/journal/stat cards. Defaults to `normal`.
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

/// Like [`intern_glyph`], but for whole resolved sets — leaked once per
/// distinct set so [`BorderGlyphs`] can carry a `Copy` reference.
fn intern_border_set(set: CustomBorderSet) -> &'static CustomBorderSet {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Vec<&'static CustomBorderSet>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .expect("border set intern lock");
    if let Some(hit) = cache.iter().find(|cached| ***cached == set) {
        return hit;
    }
    let leaked: &'static CustomBorderSet = Box::leak(Box::new(set));
    cache.push(leaked);
    leaked
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
    glyphs: ChartsGlyphsSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChartsGlyphsSection {
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
        let base = color(&surfaces.base, Color::Reset, "surfaces.base")?;
        let content = color(&surfaces.content, base, "surfaces.content")?;
        let dialog = color(&surfaces.dialog, content, "surfaces.dialog")?;
        let element = color(&surfaces.element, content, "surfaces.element")?;
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
        // Structural furniture: the divider rule and the tab separator have
        // always ridden the muted ink (dim included, as `Theme::muted` applies);
        // cards have used the normal border. Each is now a token that keeps that
        // default.
        let muted_ink = muted.add_modifier(Modifier::DIM);
        let divider = style(&borders.divider, muted_ink, "borders.divider")?;
        let card_border = style(&borders.card, border, "borders.card")?;
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
        // *lifts* the element surface rather than reusing it: element is what
        // cards/chips already sit on, so a same-color hover would be invisible
        // exactly where hover matters most — and theme files written before
        // the token existed (never overwritten on upgrade) hit this default.
        let hover = match &interaction.hover {
            Some(spec) => spec.resolve(mode, palette, "interaction.hover")?,
            None => Style::default().bg(lift(element, mode)),
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
            chart_baseline: glyph(&charts.baseline.glyph, '┈', "charts.baseline.glyph")?,
            chart_groove: glyph(&charts.glyphs.groove, '·', "charts.glyphs.groove")?,
            bar_center: glyph(&charts.glyphs.bar_center, '│', "charts.glyphs.bar_center")?,
            mood_fill: glyph(&charts.glyphs.mood_stroke, '─', "charts.glyphs.mood_stroke")?,
            scrollbar_thumb: glyph(&scrollbar_glyphs.thumb, '█', "scrollbar.glyphs.thumb")?,
            scrollbar_track: glyph(&scrollbar_glyphs.track, '║', "scrollbar.glyphs.track")?,
            scrollbar_up: glyph(&scrollbar_glyphs.up, '▲', "scrollbar.glyphs.up")?,
            scrollbar_down: glyph(&scrollbar_glyphs.down, '▼', "scrollbar.glyphs.down")?,
            borders: border_glyphs,
            focused_borders,
        };

        let status = &self.status;
        Ok(Theme {
            base,
            content,
            dialog,
            element,
            footer,
            text,
            muted,
            heading,
            placeholder,
            primary,
            secondary,
            border,
            border_subtle,
            divider,
            card_border,
            tab_separator,
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
            chrome: self.chrome.default_style,
            scrim: self.chrome.scrim,
        })
    }
}
