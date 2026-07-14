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
//! value render as plain body text on eclipse.

mod schema;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use ratatui::style::{Color, Modifier, Style};
use schema::parse;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(not(test))]
use std::sync::RwLock;

/// The bundled themes, embedded so the binary can materialize and fall back to
/// them without touching the network or the repo.
const BUNDLED: [(&str, &str); 26] = [
    ("journal", include_str!("../themes/journal.toml")),
    ("classic", include_str!("../themes/classic.toml")),
    ("eclipse", include_str!("../themes/eclipse.toml")),
    ("blossom", include_str!("../themes/blossom.toml")),
    ("fjord", include_str!("../themes/fjord.toml")),
    ("grove", include_str!("../themes/grove.toml")),
    ("tokyonight", include_str!("../themes/tokyonight.toml")),
    ("lavender", include_str!("../themes/lavender.toml")),
    ("matcha", include_str!("../themes/matcha.toml")),
    ("indigo", include_str!("../themes/indigo.toml")),
    ("maple", include_str!("../themes/maple.toml")),
    ("celadon", include_str!("../themes/celadon.toml")),
    ("rose-pine", include_str!("../themes/rose-pine.toml")),
    ("dungeon", include_str!("../themes/dungeon.toml")),
    ("synthwave", include_str!("../themes/synthwave.toml")),
    ("crt", include_str!("../themes/crt.toml")),
    ("cyberpunk", include_str!("../themes/cyberpunk.toml")),
    ("vaporwave", include_str!("../themes/vaporwave.toml")),
    ("matrix", include_str!("../themes/matrix.toml")),
    ("tron", include_str!("../themes/tron.toml")),
    ("eldritch", include_str!("../themes/eldritch.toml")),
    ("hal", include_str!("../themes/hal.toml")),
    ("gameboy", include_str!("../themes/gameboy.toml")),
    ("wasteland", include_str!("../themes/wasteland.toml")),
    ("arcade", include_str!("../themes/arcade.toml")),
    ("deep-space", include_str!("../themes/deep-space.toml")),
];

/// The theme `load` falls back to when the configured one is missing or broken.
pub(crate) const DEFAULT_THEME: &str = "journal";

/// Which variant of a `{ dark, light }` color a load resolves to. Detected from
/// the terminal background once at startup and cached for the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Dark,
    Light,
}

/// A chart fill: which glyph is repeated and how it is styled. Eclipse themes
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

/// How the reader's metadata chips (feelings, people, activities, tags) are
/// drawn: `Reversed` inverts the value's cell (the e-ink/classic look), `Bg`
/// fills with the per-category pill colors, `Bracket` is plain `[value]` text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PillStyle {
    #[default]
    Reversed,
    Bg,
    Bracket,
}

/// Which metadata chip category a pill styles — the render-side key into the
/// theme's per-category pill colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PillCategory {
    Feelings,
    People,
    Activities,
    Tags,
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

/// The resolved `[metadata]` section: pill and air-quality styles plus the
/// environment strip's glyph vocabulary. Kept behind one interned `&'static`
/// on [`Theme`] — `theme()` copies the whole struct on every call, so the
/// section adds a pointer to that copy, not two hundred bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MetadataTheme {
    pub(super) pill_style: PillStyle,
    pub(super) pill_feelings: Style,
    pub(super) pill_people: Style,
    pub(super) pill_activities: Style,
    pub(super) pill_tags: Style,
    pub(super) aqi_poor: Style,
    pub(super) aqi_very_poor: Style,
    pub(super) aqi_extremely_poor: Style,
    pub(super) pollen_high: Style,
    pub(super) mood_negative: Style,
    pub(super) mood_positive: Style,
    pub(super) glyphs: EnvGlyphs,
}

/// Like `intern_glyph` in the schema, but for whole resolved `[metadata]`
/// sections — leaked once per distinct value so [`Theme`] carries a `Copy`
/// reference. Themes resolve rarely (startup, picker preview, live reload),
/// so the linear cache scan is nothing.
/// Intern a resolved theme sub-struct by value: repeated parses (picker, live
/// reload) return the same `&'static` instead of leaking without bound, so the
/// `Copy` structs that carry it ([`Glyphs`], [`BorderGlyphs`]) stay cheap.
pub(super) fn intern<T: PartialEq + 'static>(
    value: T,
    cache: &'static std::sync::OnceLock<std::sync::Mutex<Vec<&'static T>>>,
) -> &'static T {
    let mut cache = cache
        .get_or_init(std::sync::Mutex::default)
        .lock()
        .unwrap_or_else(|_| panic!("{} intern lock", std::any::type_name::<T>()));
    if let Some(hit) = cache.iter().find(|cached| ***cached == value) {
        return hit;
    }
    let leaked: &'static T = Box::leak(Box::new(value));
    cache.push(leaked);
    leaked
}

pub(super) fn intern_metadata_theme(section: MetadataTheme) -> &'static MetadataTheme {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Vec<&'static MetadataTheme>>> = OnceLock::new();
    intern(section, &CACHE)
}

/// The eighths ramps a theme's column charts, histograms, and sparklines fill
/// with. Held behind a `&'static` (like [`MetadataTheme`]) so [`Glyphs`] stays
/// a cheap `Copy` even though the ramps are 13 chars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChartRamps {
    /// Bars growing *up* from a baseline: index 0 blank, 8 a full cell.
    pub(crate) up: [char; 9],
    /// Bars hanging *below* a baseline, quantised to the four universally-drawn
    /// upper block glyphs: index 0 blank, 3 a full cell.
    pub(crate) down: [char; 4],
}

/// Intern chart ramps by value so repeated parses (picker, live reload) don't
/// leak without bound — mirrors [`intern_metadata_theme`].
pub(super) fn intern_chart_ramps(ramps: ChartRamps) -> &'static ChartRamps {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Vec<&'static ChartRamps>>> = OnceLock::new();
    intern(ramps, &CACHE)
}

/// The markdown reader's structural chrome — the code-fence frame and the
/// quote/code left rails. Multi-character (a rail is `│ `, a fence corner `╭─`),
/// so held behind a `&'static` to keep [`Glyphs`] a cheap `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkdownGlyphs {
    /// The left rail of a blockquote (`markdown.glyphs.quote_rail`).
    pub(crate) quote_rail: String,
    /// The left rail of a fenced code block (`markdown.glyphs.code_rail`).
    pub(crate) code_rail: String,
    /// The top of a code fence, before the language label (`markdown.glyphs.code_top`).
    pub(crate) code_top: String,
    /// The bottom of a code fence (`markdown.glyphs.code_bottom`).
    pub(crate) code_bottom: String,
    /// The unordered-list bullet (`markdown.glyphs.bullet`).
    pub(crate) bullet: char,
    /// The done / to-do task checkboxes (`markdown.glyphs.task_done` / `task_todo`).
    pub(crate) task_done: String,
    pub(crate) task_todo: String,
    /// The GitHub-alert icons (`[markdown.glyphs.alert]`).
    pub(crate) alert: AlertGlyphs,
}

/// The icon leading each GitHub-style alert blockquote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct AlertGlyphs {
    pub(crate) note: char,
    pub(crate) tip: char,
    pub(crate) important: char,
    pub(crate) warning: char,
    pub(crate) caution: char,
}

/// Intern markdown glyphs by value — mirrors [`intern_metadata_theme`].
pub(super) fn intern_markdown_glyphs(glyphs: MarkdownGlyphs) -> &'static MarkdownGlyphs {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Vec<&'static MarkdownGlyphs>>> = OnceLock::new();
    intern(glyphs, &CACHE)
}

/// The environment strip's glyphs (`[metadata.glyphs]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EnvGlyphs {
    /// The full-width rule above the metadata block (`metadata.glyphs.rule`).
    pub(crate) rule: char,
    /// The dot between strip items; always rendered with a space each side so
    /// the strip's width math stays fixed.
    pub(crate) separator: char,
    /// The marker leading the location item.
    pub(crate) location: char,
    /// The sunrise marker inside the sun item.
    pub(crate) sunrise: char,
    /// The sunset marker inside the sun item.
    pub(crate) sunset: char,
    /// The dot leading the air-quality badge.
    pub(crate) aqi: char,
    /// The marker leading the high-pollen badge.
    pub(crate) pollen: char,
    /// The mood bar's filled cells; the valence hue rides
    /// `metadata.environment.mood_negative`/`mood_positive`.
    pub(crate) mood_fill: char,
    /// The mood bar's empty cells. The center marker stays the shared
    /// `charts.glyphs.diverge_center` (its heavy at-zero variant is code-side —
    /// weight is the meaning).
    pub(crate) mood_track: char,
    /// The glyph leading each chip pill, by category.
    pub(crate) feelings: char,
    pub(crate) people: char,
    pub(crate) activities: char,
    pub(crate) tags: char,
    /// The weather glyph per condition slug (`[metadata.glyphs.weather]`).
    pub(crate) weather: WeatherGlyphs,
    /// The moon glyph per phase slug (`[metadata.glyphs.moon]`).
    pub(crate) moon: MoonGlyphs,
}

/// The environment strip's weather glyph per condition slug the context
/// provider emits (`[metadata.glyphs.weather]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WeatherGlyphs {
    pub(super) clear: char,
    pub(super) mostly_clear: char,
    pub(super) partly_cloudy: char,
    pub(super) cloudy: char,
    pub(super) fog: char,
    pub(super) drizzle: char,
    pub(super) rain: char,
    pub(super) snow: char,
    pub(super) thunderstorm: char,
}

impl WeatherGlyphs {
    /// The glyph for a stored condition slug; `None` for slugs this build
    /// doesn't know (future providers), which render without a glyph.
    pub(crate) fn for_slug(self, slug: &str) -> Option<char> {
        Some(match slug {
            "clear" => self.clear,
            "mostly-clear" => self.mostly_clear,
            "partly-cloudy" => self.partly_cloudy,
            "cloudy" => self.cloudy,
            "fog" => self.fog,
            "drizzle" => self.drizzle,
            "rain" => self.rain,
            "snow" => self.snow,
            "thunderstorm" => self.thunderstorm,
            _ => return None,
        })
    }
}

/// The environment strip's moon glyph per phase slug the celestial provider
/// emits (`[metadata.glyphs.moon]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MoonGlyphs {
    pub(super) new: char,
    pub(super) waxing_crescent: char,
    pub(super) first_quarter: char,
    pub(super) waxing_gibbous: char,
    pub(super) full: char,
    pub(super) waning_gibbous: char,
    pub(super) last_quarter: char,
    pub(super) waning_crescent: char,
}

impl MoonGlyphs {
    /// The glyph for a stored phase slug; `None` for unknown slugs.
    pub(crate) fn for_slug(self, slug: &str) -> Option<char> {
        Some(match slug {
            "new" => self.new,
            "waxing-crescent" => self.waxing_crescent,
            "first-quarter" => self.first_quarter,
            "waxing-gibbous" => self.waxing_gibbous,
            "full" => self.full,
            "waning-gibbous" => self.waning_gibbous,
            "last-quarter" => self.last_quarter,
            "waning-crescent" => self.waning_crescent,
            _ => return None,
        })
    }
}

/// The theme's identity glyphs — every meaning-free character the UI repeats.
/// Meaning-carrying glyph *variance* (heavy vs light at a zero mood, distinct
/// chart-series glyphs) stays in code and [`Fill`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Glyphs {
    /// The stripe down a focused panel's left edge (flat chrome).
    pub(crate) focus_stripe: char,
    /// The accent edges of a toast card (flat chrome).
    pub(crate) toast_edge: char,
    /// The dismissal countdown line along a toast's bottom edge; the filled
    /// span shrinks as the toast nears its deadline.
    pub(crate) toast_progress: char,
    /// The separator between tab labels; always rendered with a space each
    /// side so the strip's width math stays fixed.
    pub(crate) tab_separator: char,
    /// The rule of section dividers (month headers, "Archived").
    pub(crate) divider: char,
    /// The plain full-width rule separating dialog sections (`borders.glyphs.separator`).
    pub(crate) separator: char,
    /// The zero-line tick shown in the gaps/edges of a signed column chart
    /// (`charts.glyphs.baseline`).
    pub(crate) chart_baseline: char,
    /// The zero-line drawn directly under each column (`charts.glyphs.rule`).
    pub(crate) chart_rule: char,
    /// The empty cell of a diverging (Δ / mood) bar (`charts.glyphs.diverge_track`).
    pub(crate) diverge_track: char,
    /// The center pivot of a diverging bar (`charts.glyphs.diverge_center`). The
    /// heavy variant shown at an exact zero stays code-side (weight carries
    /// meaning).
    pub(crate) diverge_center: char,
    /// The eighths ramps for vertical bars (`charts.glyphs.ramp_up`/`ramp_down`).
    pub(crate) ramps: &'static ChartRamps,
    /// The scrollbar's draggable handle (`glyphs.scrollbar_thumb`).
    pub(crate) scrollbar_thumb: char,
    /// The scrollbar's track behind the handle (`glyphs.scrollbar_track`).
    pub(crate) scrollbar_track: char,
    /// The arrow capping the scrollbar's top (`glyphs.scrollbar_up`).
    pub(crate) scrollbar_up: char,
    /// The arrow capping the scrollbar's bottom (`glyphs.scrollbar_down`).
    pub(crate) scrollbar_down: char,
    /// The disclosure marker for an expanded/collapsed group (`indicators.glyphs`).
    pub(crate) expanded: char,
    pub(crate) collapsed: char,
    /// The marker trailing a starred entry (`indicators.glyphs.starred`).
    pub(crate) starred: char,
    /// The multi-character markdown chrome (fence frame, quote/code rails).
    pub(crate) markdown: &'static MarkdownGlyphs,
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
    base: Color,
    content: Color,
    dialog: Color,
    raised: Color,
    footer: Color,
    text: Style,
    muted: Style,
    heading: Style,
    placeholder: Style,
    primary: Style,
    secondary: Style,
    border_subtle: Style,
    border_active: Style,
    border_inactive: Style,
    divider: Style,
    card_border: Style,
    tab_separator: Style,
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
    chart_bar: Fill,
    chart_track: Fill,
    chart_baseline: Style,
    chart_label: Style,
    md_heading: Style,
    md_heading2: Style,
    md_subheading: Style,
    md_link: Style,
    md_code: Style,
    md_inline_code: Style,
    md_blockquote: Style,
    md_highlight: Style,
    syntax: Syntax,
    metadata: &'static MetadataTheme,
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
    // Install the configured theme so something is themed before the App exists.
    // The App re-resolves the effective theme (accounting for a journal-specific
    // override) in `apply_effective_theme` during construction and drives the
    // toast from there, so this transient pre-App load stays silent.
    let (theme, _) = load(config_path, &ui.theme, mode());
    install(theme);
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

/// The resolved bundled eclipse theme, for tests asserting the monochrome
/// glyph-differentiation contract end to end.
#[cfg(test)]
pub(crate) fn test_eclipse_theme() -> Theme {
    builtin("eclipse", Mode::Dark).expect("bundled eclipse theme resolves")
}

/// Resolve a theme snippet (dark mode) for tests that pin specific tokens.
#[cfg(test)]
pub(crate) fn test_theme_from_toml(text: &str) -> Theme {
    parse(&format!("schema_version = 1\n{text}"), Mode::Dark).expect("test theme snippet resolves")
}

impl Theme {
    /// The look the app has always had on a bare terminal: default colors,
    /// bordered chrome, meaning carried by modifiers. Resolved from the
    /// bundled `classic.toml` — the e-ink/no-assumptions theme, which also
    /// swaps the metadata glyphs to ASCII — so the fallback for a missing or
    /// broken theme never assumes more than a bare terminal renders. A test
    /// pins the two to each other in both modes.
    pub(crate) fn terminal_default() -> Self {
        static DEFAULT: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
        *DEFAULT
            .get_or_init(|| builtin("classic", Mode::Dark).expect("bundled classic theme resolves"))
    }

    // --- surfaces ---

    /// The bottom surface layer, painted under every frame: app margins,
    /// full-screen modal screens, and the footer's default.
    pub(crate) fn base_bg(self) -> Color {
        self.base
    }

    /// The main content panels (entries, journals, insights, the entry viewer),
    /// and toasts on bordered chrome. Defaults to the base surface.
    pub(crate) fn content_bg(self) -> Color {
        self.content
    }

    /// Dialog surfaces, defaulting to the content surface unless a theme splits them.
    pub(crate) fn dialog_bg(self) -> Color {
        self.dialog
    }

    /// Raised items sitting on a panel: inputs, cards, list rows, status bars.
    pub(crate) fn raised_bg(self) -> Color {
        self.raised
    }

    /// The hint/footer bar. Defaults to the base surface, so a theme can tint
    /// the footer separately or leave it flush with the backdrop.
    pub(crate) fn footer_bg(self) -> Color {
        self.footer
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
    /// ink unless a theme sets `tabs.separator`.
    pub(crate) fn tab_separator(self) -> Style {
        self.tab_separator
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
    /// `borders.divider`.
    pub(crate) fn divider(self) -> Style {
        self.divider
    }

    /// A recessed box outline — a touch brighter than [`Self::faint_rule`] so
    /// card and panel borders read as present-but-quiet. Defaults to the normal
    /// border unless a theme sets `borders.card`.
    pub(crate) fn card_border(self) -> Style {
        self.card_border
    }

    /// The scrollbar's draggable thumb. Recedes when its panel is unfocused,
    /// mirroring how the border quiets — so a background panel's bar doesn't
    /// compete with the focused one.
    pub(crate) fn scrollbar_thumb(self, focused: bool) -> Style {
        Self::recede_scrollbar(self.scrollbar_thumb, focused)
    }

    /// The scrollbar's track behind the thumb.
    pub(crate) fn scrollbar_track(self, focused: bool) -> Style {
        Self::recede_scrollbar(self.scrollbar_track, focused)
    }

    /// The scrollbar's up/down arrow caps. Defaults to the thumb hue unless a
    /// theme sets `scrollbar.arrow`.
    pub(crate) fn scrollbar_arrow(self, focused: bool) -> Style {
        Self::recede_scrollbar(self.scrollbar_arrow, focused)
    }

    /// Dim a scrollbar style for an unfocused panel. Drops any bold weight and
    /// adds `DIM` so the bar visibly recedes even under the terminal-default
    /// theme, where the parts carry no colour of their own.
    fn recede_scrollbar(style: Style, focused: bool) -> Style {
        if focused {
            style
        } else {
            style
                .remove_modifier(Modifier::BOLD)
                .add_modifier(Modifier::DIM)
        }
    }

    // --- charts ---

    /// The filled part of count/frequency bars.
    pub(crate) fn chart_bar(self) -> Fill {
        self.chart_bar
    }

    /// The empty remainder of a bar.
    pub(crate) fn chart_track(self) -> Fill {
        self.chart_track
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

    /// The top-level markdown heading (H1) in the entry viewer.
    pub(crate) fn md_heading(self) -> Style {
        self.md_heading
    }

    /// The second-level markdown heading (H2), defaulting to `md_heading` so
    /// H1 and H2 read alike until a theme splits them.
    pub(crate) fn md_heading2(self) -> Style {
        self.md_heading2
    }

    /// Faded markdown sub-headings (H3 and deeper), for themes that step down
    /// the hierarchy.
    pub(crate) fn md_subheading(self) -> Style {
        self.md_subheading
    }

    /// Markdown links.
    pub(crate) fn md_link(self) -> Style {
        self.md_link
    }

    /// Fenced code blocks.
    pub(crate) fn md_code(self) -> Style {
        self.md_code
    }

    /// Inline `` `code` `` spans, defaulting to `md_code`.
    pub(crate) fn md_inline_code(self) -> Style {
        self.md_inline_code
    }

    /// Block quotes.
    pub(crate) fn md_blockquote(self) -> Style {
        self.md_blockquote
    }

    /// `==highlight==` spans, defaulting to the primary accent (reversed + bold).
    pub(crate) fn md_highlight(self) -> Style {
        self.md_highlight
    }

    /// Syntax-highlight colors for fenced code blocks.
    pub(crate) fn syntax(self) -> Syntax {
        self.syntax
    }

    // --- entry metadata ---

    /// How the reader's metadata chips are drawn (`metadata.pills.style`).
    pub(crate) fn pill_style(self) -> PillStyle {
        self.metadata.pill_style
    }

    /// The style layered onto one metadata pill. `Reversed` inversion is
    /// code-enforced (monochrome contract) and ignores the category colors;
    /// `Bracket` pills are plain text; `Bg` uses the per-category pill styles,
    /// which layer under the value's own ink like hover does.
    pub(crate) fn pill(self, category: PillCategory) -> Style {
        match self.metadata.pill_style {
            PillStyle::Reversed => Style::default().add_modifier(Modifier::REVERSED),
            PillStyle::Bracket => Style::default(),
            PillStyle::Bg => match category {
                PillCategory::Feelings => self.metadata.pill_feelings,
                PillCategory::People => self.metadata.pill_people,
                PillCategory::Activities => self.metadata.pill_activities,
                PillCategory::Tags => self.metadata.pill_tags,
            },
        }
    }

    /// The glyph leading a chip pill of the given category, so the pill row
    /// echoes the environment strip's glyph-led grammar.
    pub(crate) fn pill_glyph(self, category: PillCategory) -> char {
        match category {
            PillCategory::Feelings => self.metadata.glyphs.feelings,
            PillCategory::People => self.metadata.glyphs.people,
            PillCategory::Activities => self.metadata.glyphs.activities,
            PillCategory::Tags => self.metadata.glyphs.tags,
        }
    }

    /// The style of the air-quality badge for a European AQI reading, or
    /// `None` below 60 — clean air never renders. Bands: 60–80 poor, 80–100
    /// very poor, 100+ extremely poor (bold in code so the worst band survives
    /// monochrome).
    pub(crate) fn aqi_band(self, aqi: i64) -> Option<Style> {
        if aqi < 60 {
            None
        } else if aqi < 80 {
            Some(self.metadata.aqi_poor)
        } else if aqi < 100 {
            Some(self.metadata.aqi_very_poor)
        } else {
            Some(
                self.metadata
                    .aqi_extremely_poor
                    .add_modifier(Modifier::BOLD),
            )
        }
    }

    /// The style of the strip's high-pollen badge — like the AQI bands it
    /// only renders when there is something to warn about, so it defaults to
    /// the warning hue.
    pub(crate) fn pollen_high(self) -> Style {
        self.metadata.pollen_high
    }

    /// The mood gauge's filled-cell style for a valence: negative fills read as
    /// the theme's error hue, positive as its success hue.
    pub(crate) fn mood_fill(self, positive: bool) -> Style {
        if positive {
            self.metadata.mood_positive
        } else {
            self.metadata.mood_negative
        }
    }

    /// The environment strip's glyph vocabulary (`[metadata.glyphs]`).
    pub(crate) fn env_glyphs(self) -> EnvGlyphs {
        self.metadata.glyphs
    }

    // --- glyphs ---

    /// The theme's identity glyphs.
    pub(crate) fn glyphs(self) -> Glyphs {
        self.glyphs
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
            ("chart_bar", self.chart_bar.style),
            ("chart_track", self.chart_track.style),
            ("chart_baseline", self.chart_baseline),
            ("chart_label", self.chart_label),
            ("md_heading", self.md_heading),
            ("md_heading2", self.md_heading2),
            ("md_subheading", self.md_subheading),
            ("md_link", self.md_link),
            ("md_code", self.md_code),
            ("md_inline_code", self.md_inline_code),
            ("md_blockquote", self.md_blockquote),
            ("md_highlight", self.md_highlight),
            ("pill_feelings", self.metadata.pill_feelings),
            ("pill_people", self.metadata.pill_people),
            ("pill_activities", self.metadata.pill_activities),
            ("pill_tags", self.metadata.pill_tags),
            ("aqi_poor", self.metadata.aqi_poor),
            ("aqi_very_poor", self.metadata.aqi_very_poor),
            ("aqi_extremely_poor", self.metadata.aqi_extremely_poor),
            ("mood_negative", self.metadata.mood_negative),
            ("mood_positive", self.metadata.mood_positive),
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

/// Theme names already warned about this session. Keyed by name so moving
/// between journals that resolve to the same broken theme toasts only once; a
/// clean load of that name clears it, so a later fix-then-break warns again.
static WARNED_THEMES: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Record the effective theme's load result. The first time a given theme name
/// fails this session it reports one warning through the shared notification
/// queue for the event loop to toast; re-applying the same broken theme (journal
/// switches, background snapshots) stays silent. A clean load clears the name so
/// a genuine later break can warn again.
pub(crate) fn note_theme_load_warning(name: &str, warning: Option<String>) {
    let mut warned = WARNED_THEMES.lock().expect("warned themes lock");
    match warning {
        Some(msg) if warned.insert(name.to_string()) => {
            crate::tui::state::report_notification(crate::tui::state::ToastVariant::Warning, msg);
        }
        Some(_) => {}
        None => {
            warned.remove(name);
        }
    }
}

/// The toast text shown when the configured theme can't be loaded and the app
/// falls back to the default. Only reached from the non-test theme-install path.
#[cfg(not(test))]
pub(crate) fn format_theme_warning(name: &str, err: &anyhow::Error) -> String {
    format!(
        "Theme '{name}' couldn't load ({}); using default",
        crate::tui::concise_error(err)
    )
}

/// Load the named theme, materializing the bundled files first. On any failure
/// (missing file, bad TOML, unknown color) returns the built-in
/// [`DEFAULT_THEME`] alongside the error so the caller can surface it — the app
/// always starts.
pub(crate) fn load(config_path: &Path, name: &str, mode: Mode) -> (Theme, Option<anyhow::Error>) {
    match try_load(config_path, name, mode) {
        Ok(theme) => (theme, None),
        Err(err) => (
            builtin(DEFAULT_THEME, mode).unwrap_or_else(Theme::terminal_default),
            Some(err),
        ),
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
