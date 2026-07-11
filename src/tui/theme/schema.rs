//! The theme file's TOML schema and its resolution into a [`Theme`]: serde
//! section structs, palette and color parsing, and the per-token defaults and
//! inheritance applied by [`ThemeFile::resolve`].

use anyhow::{Context, Result, anyhow, bail};
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::{collections::BTreeMap, str::FromStr};

use super::{BorderGlyphs, ChromeStyle, Fill, Glyphs, Mode, Syntax, Theme};

pub(super) fn parse(text: &str, mode: Mode) -> Result<Theme> {
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
    pub(super) fn resolve(&self, mode: Mode) -> Result<Theme> {
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
