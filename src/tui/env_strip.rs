//! The reader's environment strip: weather, air quality, moon, sun, and
//! location as compact glyph-led items that flow under the metadata
//! separator. Item building is width-independent; [`env_strip_rows`] flows
//! items into rows for one width, and is the single source both the height
//! calculation and the two render modes consume, so layout and paint can
//! never disagree.

use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use notema_domain::{AirQuality, Celestial, Weather};

use crate::tui::{entry_rows::wrap_text_hanging, theme::theme};

/// The cells `" · "` occupies between two items on a row.
const SEPARATOR_WIDTH: u16 = 3;

/// The hanging indent of a wrapped item's continuation rows, matching the
/// two cells its glyph-plus-space lead occupies.
const CONTINUATION_INDENT: u16 = 2;

/// One glyph-led item of the environment strip. Items never break mid-item
/// when the strip wraps; an item wider than the whole strip is pre-split by
/// [`env_strip_rows`] into continuation items instead.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EnvItem {
    pub(crate) glyph: Option<char>,
    pub(crate) glyph_style: Style,
    /// The item's text as styled runs, so an inner marker (the sun item's
    /// sunset glyph) can carry the accent while the rest stays ink.
    pub(crate) segments: Vec<(String, Style)>,
    /// A continuation row of a wrapped item: indented under the glyph and
    /// never preceded by a separator.
    pub(crate) continuation: bool,
}

impl EnvItem {
    fn new(glyph: Option<char>, glyph_style: Style, text: String, style: Style) -> Self {
        Self::with_segments(glyph, glyph_style, vec![(text, style)])
    }

    fn with_segments(
        glyph: Option<char>,
        glyph_style: Style,
        segments: Vec<(String, Style)>,
    ) -> Self {
        Self {
            glyph,
            glyph_style,
            segments,
            continuation: false,
        }
    }

    /// The item's text with the segment styling flattened away.
    pub(crate) fn text(&self) -> String {
        self.segments
            .iter()
            .map(|(text, _)| text.as_str())
            .collect()
    }

    /// The display cells the item occupies: the glyph and its trailing space,
    /// or the continuation indent, plus the text.
    pub(crate) fn width(&self) -> u16 {
        let lead: u16 = if self.glyph.is_some() || self.continuation {
            2
        } else {
            0
        };
        let text = self
            .segments
            .iter()
            .map(|(text, _)| UnicodeWidthStr::width(text.as_str()))
            .sum::<usize>()
            .min(u16::MAX as usize) as u16;
        lead.saturating_add(text)
    }
}

/// Build the strip's items from an entry's context data, in display order:
/// weather, air quality, moon, sun, location. Absent data simply yields no
/// item, so a location-only entry gets a location-only strip.
pub(crate) fn environment_items(
    location: Option<&str>,
    weather: Option<&Weather>,
    celestial: Option<&Celestial>,
    air: Option<&AirQuality>,
) -> Vec<EnvItem> {
    let glyphs = theme().env_glyphs();
    let accent = theme().secondary();
    let ink = theme().muted();
    let mut items = Vec::new();

    if let Some(weather) = weather {
        let glyph = weather
            .condition
            .as_deref()
            .and_then(|slug| glyphs.weather.for_slug(slug));
        let mut parts = Vec::new();
        if let Some(temperature) = weather.temperature_celsius {
            // Feels-like folds into the temperature so a wrap can never strand
            // a bare "(feels …)" fragment on its own row.
            match weather.feels_like_celsius {
                Some(feels) if (feels - temperature).abs() > 3.0 => {
                    parts.push(format!("{temperature:.0}°C (feels {feels:.0}°C)"));
                }
                _ => parts.push(format!("{temperature:.0}°C")),
            }
        }
        if let Some(condition) = weather.condition.as_deref() {
            parts.push(humanize_slug(condition));
        }
        if !parts.is_empty() {
            items.push(EnvItem::new(glyph, accent, parts.join(" "), ink));
        }
    }

    if let Some(aqi) = air.and_then(|air| air.european_aqi)
        && let Some(band) = theme().aqi_band(aqi)
    {
        items.push(EnvItem::new(
            Some(glyphs.aqi),
            band,
            format!("AQI {aqi}"),
            band,
        ));
    }

    // Pollen follows the AQI badge's gate philosophy: quiet until a species
    // reaches its "high" band, then a warning-hued item naming it.
    if let Some(air) = air
        && let Some(text) = high_pollen_text(air)
    {
        let style = theme().pollen_high();
        items.push(EnvItem::new(Some(glyphs.pollen), style, text, style));
    }

    if let Some(celestial) = celestial {
        let slug = celestial.moon_phase_name.clone().or_else(|| {
            celestial
                .moon_phase
                .map(|fraction| moon_phase_slug(fraction).to_string())
        });
        if let Some(slug) = slug {
            items.push(EnvItem::new(
                glyphs.moon.for_slug(&slug),
                accent,
                // "full moon", not a bare "full" — the phase name alone
                // doesn't say what it names, least of all in ASCII glyph sets.
                format!("{} moon", humanize_slug(&slug)),
                ink,
            ));
        }

        let sunrise = celestial.sunrise.as_deref().and_then(local_time);
        let sunset = celestial.sunset.as_deref().and_then(local_time);
        // Sunrise and sunset are one atomic item so a wrap can't separate
        // them; the inner sunset marker rides its own accent segment so both
        // sun glyphs theme alike.
        let sun = match (sunrise, sunset) {
            (Some(rise), Some(set)) => Some(EnvItem::with_segments(
                Some(glyphs.sunrise),
                accent,
                vec![
                    (format!("{rise} "), ink),
                    // Glyph-plus-space, matching the sunrise lead's spacing.
                    (format!("{} ", glyphs.sunset), accent),
                    (set, ink),
                ],
            )),
            (Some(rise), None) => Some(EnvItem::new(Some(glyphs.sunrise), accent, rise, ink)),
            (None, Some(set)) => Some(EnvItem::new(Some(glyphs.sunset), accent, set, ink)),
            (None, None) => None,
        };
        items.extend(sun);
    }

    if let Some(location) = location {
        items.push(EnvItem::new(
            Some(glyphs.location),
            accent,
            location.to_string(),
            ink,
        ));
    }

    items
}

/// Flow the items into rows of `width` cells: greedy, `" · "` between items,
/// never breaking mid-item. An item wider than the whole strip word-wraps
/// into continuation items indented under its glyph.
pub(crate) fn env_strip_rows(width: u16, items: &[EnvItem]) -> Vec<Vec<EnvItem>> {
    if width == 0 || items.is_empty() {
        return Vec::new();
    }

    let mut rows: Vec<Vec<EnvItem>> = Vec::new();
    let mut row: Vec<EnvItem> = Vec::new();
    let mut row_width: u16 = 0;
    for item in items {
        for item in split_oversize(width, item) {
            let separator = if row.is_empty() { 0 } else { SEPARATOR_WIDTH };
            if !row.is_empty()
                && (item.continuation || row_width + separator + item.width() > width)
            {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            row_width += if row.is_empty() { 0 } else { SEPARATOR_WIDTH } + item.width();
            row.push(item);
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// The rows the strip occupies at `width` — the height half of
/// [`env_strip_rows`], used by the metadata section's height math.
pub(crate) fn env_strip_height(width: u16, items: &[EnvItem]) -> u16 {
    env_strip_rows(width, items).len().min(u16::MAX as usize) as u16
}

/// Word-wrap an item wider than the whole strip into a leading chunk plus
/// indented continuation chunks; items that fit pass through unchanged.
/// Wrapping flattens the item to one styled run — only the single-segment
/// location item ever grows past a strip width in practice.
fn split_oversize(width: u16, item: &EnvItem) -> Vec<EnvItem> {
    if item.width() <= width {
        return vec![item.clone()];
    }
    let lead = if item.glyph.is_some() { 2u16 } else { 0 };
    let first = (width.saturating_sub(lead)).max(1) as usize;
    let rest = (width.saturating_sub(CONTINUATION_INDENT)).max(1) as usize;
    let style = item
        .segments
        .first()
        .map(|(_, style)| *style)
        .unwrap_or_default();
    wrap_text_hanging(&item.text(), first, rest)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| EnvItem {
            glyph: if index == 0 { item.glyph } else { None },
            glyph_style: item.glyph_style,
            segments: vec![(chunk, style)],
            continuation: index > 0,
        })
        .collect()
}

/// Daily-mean grains/m³ from which a species counts as "high". European
/// allergy services differ by a few grains per species; these sit in the
/// middle of the published bands.
const BIRCH_POLLEN_HIGH: f64 = 100.0;
const GRASS_POLLEN_HIGH: f64 = 30.0;
const RAGWEED_POLLEN_HIGH: f64 = 15.0;

/// The high-pollen badge's text, or `None` while every species sits below its
/// "high" band — unremarkable pollen never renders. One high species is named;
/// several fold into a plain "high pollen".
fn high_pollen_text(air: &AirQuality) -> Option<String> {
    let high: Vec<&str> = [
        ("birch", air.birch_pollen, BIRCH_POLLEN_HIGH),
        ("grass", air.grass_pollen, GRASS_POLLEN_HIGH),
        ("ragweed", air.ragweed_pollen, RAGWEED_POLLEN_HIGH),
    ]
    .into_iter()
    .filter(|(_, count, threshold)| count.is_some_and(|count| count >= *threshold))
    .map(|(species, _, _)| species)
    .collect();
    match high.as_slice() {
        [] => None,
        [species] => Some(format!("high {species} pollen")),
        _ => Some("high pollen".to_string()),
    }
}

/// A stored kebab-case slug as strip text: `"partly-cloudy"` → `"partly cloudy"`.
fn humanize_slug(slug: &str) -> String {
    slug.replace('-', " ")
}

/// The named phase for a `[0, 1)` cycle fraction, mirroring the bucketing the
/// celestial provider writes — for imported entries that carry only the
/// numeric fraction.
fn moon_phase_slug(fraction: f64) -> &'static str {
    match (fraction * 8.0).round() as i64 % 8 {
        0 => "new",
        1 => "waxing-crescent",
        2 => "first-quarter",
        3 => "waxing-gibbous",
        4 => "full",
        5 => "waning-gibbous",
        6 => "last-quarter",
        _ => "waning-crescent",
    }
}

/// An RFC3339 timestamp as wall-clock `HH:MM` at its own stored offset — the
/// entry's local time, no zone math.
fn local_time(rfc3339: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|instant| instant.format("%H:%M").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather(condition: Option<&str>, temperature: Option<f64>, feels: Option<f64>) -> Weather {
        Weather {
            condition: condition.map(str::to_string),
            temperature_celsius: temperature,
            feels_like_celsius: feels,
            ..Weather::default()
        }
    }

    fn air(european_aqi: Option<i64>) -> AirQuality {
        AirQuality {
            european_aqi,
            ..AirQuality::default()
        }
    }

    // Glyph expectations pin the fallback theme's set — classic's all-ASCII
    // overrides, since no theme is installed under test.
    fn texts(items: &[EnvItem]) -> Vec<String> {
        items.iter().map(EnvItem::text).collect()
    }

    #[test]
    fn empty_inputs_yield_no_items() {
        assert!(environment_items(None, None, None, None).is_empty());
        assert!(environment_items(None, Some(&Weather::default()), None, None).is_empty());
        assert!(env_strip_rows(40, &[]).is_empty());
        assert_eq!(env_strip_height(40, &[]), 0);
    }

    #[test]
    fn weather_item_combines_glyph_temperature_and_condition() {
        let items = environment_items(
            None,
            Some(&weather(Some("partly-cloudy"), Some(18.4), None)),
            None,
            None,
        );
        assert_eq!(texts(&items), ["18°C partly cloudy"]);
        assert_eq!(items[0].glyph, Some('~'));

        // Temperature-only and condition-only halves still render.
        let temp_only = environment_items(None, Some(&weather(None, Some(-3.6), None)), None, None);
        assert_eq!(texts(&temp_only), ["-4°C"]);
        assert_eq!(temp_only[0].glyph, None);
        let condition_only =
            environment_items(None, Some(&weather(Some("fog"), None, None)), None, None);
        assert_eq!(texts(&condition_only), ["fog"]);

        // Unknown future slugs render their text without a glyph.
        let unknown = environment_items(
            None,
            Some(&weather(Some("hail"), Some(1.0), None)),
            None,
            None,
        );
        assert_eq!(texts(&unknown), ["1°C hail"]);
        assert_eq!(unknown[0].glyph, None);
    }

    #[test]
    fn feels_like_folds_in_only_past_three_degrees() {
        let close = environment_items(
            None,
            Some(&weather(None, Some(18.0), Some(15.0))),
            None,
            None,
        );
        assert_eq!(texts(&close), ["18°C"], "a 3.0°C gap must stay quiet");
        let far = environment_items(
            None,
            Some(&weather(None, Some(18.0), Some(14.9))),
            None,
            None,
        );
        assert_eq!(texts(&far), ["18°C (feels 15°C)"]);
    }

    #[test]
    fn aqi_gates_below_sixty_and_carries_its_band_style() {
        let clean = environment_items(None, None, None, Some(&air(Some(59))));
        assert!(clean.is_empty(), "clean air must never render");
        assert!(environment_items(None, None, None, Some(&air(None))).is_empty());

        let poor = environment_items(None, None, None, Some(&air(Some(72))));
        assert_eq!(texts(&poor), ["AQI 72"]);
        assert_eq!(poor[0].segments[0].1, theme().aqi_band(72).unwrap());
        assert_eq!(poor[0].glyph, Some('!'));
    }

    #[test]
    fn pollen_gates_at_high_and_names_the_species() {
        let pollen = |birch: Option<f64>, grass: Option<f64>, ragweed: Option<f64>| AirQuality {
            birch_pollen: birch,
            grass_pollen: grass,
            ragweed_pollen: ragweed,
            ..AirQuality::default()
        };

        // Below every "high" band (or absent, as outside Europe) — no item.
        let calm = environment_items(
            None,
            None,
            None,
            Some(&pollen(Some(99.0), Some(29.0), None)),
        );
        assert!(calm.is_empty(), "unremarkable pollen must never render");
        assert!(environment_items(None, None, None, Some(&pollen(None, None, None))).is_empty());

        let birch = environment_items(None, None, None, Some(&pollen(Some(100.0), None, None)));
        assert_eq!(texts(&birch), ["high birch pollen"]);
        assert_eq!(birch[0].glyph, Some('%'));
        assert_eq!(birch[0].glyph_style, theme().pollen_high());

        // Several high species fold into one unnamed badge.
        let several = environment_items(
            None,
            None,
            None,
            Some(&pollen(None, Some(30.0), Some(15.0))),
        );
        assert_eq!(texts(&several), ["high pollen"]);

        // Pollen rides behind the AQI badge when both warrant an item.
        let with_aqi = environment_items(
            None,
            None,
            None,
            Some(&AirQuality {
                european_aqi: Some(72),
                grass_pollen: Some(80.0),
                ..AirQuality::default()
            }),
        );
        assert_eq!(texts(&with_aqi), ["AQI 72", "high grass pollen"]);
    }

    #[test]
    fn moon_uses_the_stored_name_or_derives_it_from_the_fraction() {
        let named = environment_items(
            None,
            None,
            Some(&Celestial {
                moon_phase_name: Some("waxing-gibbous".into()),
                ..Celestial::default()
            }),
            None,
        );
        assert_eq!(texts(&named), ["waxing gibbous moon"]);

        let derived = environment_items(
            None,
            None,
            Some(&Celestial {
                moon_phase: Some(0.5),
                ..Celestial::default()
            }),
            None,
        );
        assert_eq!(texts(&derived), ["full moon"]);
        assert_eq!(derived[0].glyph, Some('O'));

        // The derivation mirrors the provider's bucketing at its boundaries.
        assert_eq!(moon_phase_slug(0.0), "new");
        assert_eq!(moon_phase_slug(0.9999), "new");
        assert_eq!(moon_phase_slug(0.0625), "waxing-crescent");
        assert_eq!(moon_phase_slug(0.25), "first-quarter");
        assert_eq!(moon_phase_slug(0.875), "waning-crescent");
    }

    #[test]
    fn sun_times_render_local_wall_clock_as_one_item() {
        let both = environment_items(
            None,
            None,
            Some(&Celestial {
                sunrise: Some("2026-07-14T05:12:30+02:00".into()),
                sunset: Some("2026-07-14T21:48:02+02:00".into()),
                ..Celestial::default()
            }),
            None,
        );
        assert_eq!(texts(&both), ["05:12 v 21:48"]);
        assert_eq!(both[0].glyph, Some('^'));
        // The inner sunset marker rides its own accent segment, matching the
        // leading sunrise glyph's styling.
        assert_eq!(both[0].segments[1], ("v ".to_string(), theme().secondary()));
        assert_eq!(both[0].segments[0].1, theme().muted());

        let sunset_only = environment_items(
            None,
            None,
            Some(&Celestial {
                sunset: Some("2026-07-14T21:48:02+02:00".into()),
                ..Celestial::default()
            }),
            None,
        );
        assert_eq!(texts(&sunset_only), ["21:48"]);
        assert_eq!(sunset_only[0].glyph, Some('v'));

        // An unparsable timestamp drops its half instead of rendering garbage.
        let broken = environment_items(
            None,
            None,
            Some(&Celestial {
                sunrise: Some("yesterday-ish".into()),
                ..Celestial::default()
            }),
            None,
        );
        assert!(broken.is_empty());
    }

    #[test]
    fn location_is_the_final_item() {
        let items = environment_items(
            Some("Berlin, Germany"),
            Some(&weather(Some("clear"), Some(21.0), None)),
            None,
            None,
        );
        assert_eq!(texts(&items), ["21°C clear", "Berlin, Germany"]);
        let location = items.last().unwrap();
        assert_eq!(location.glyph, Some('@'));
        // Every leading glyph shares the one accent — no item's marker may
        // read darker or brighter than its neighbours'.
        assert_eq!(location.glyph_style, theme().secondary());
        assert_eq!(items[0].glyph_style, theme().secondary());
    }

    #[test]
    fn rows_flow_greedily_and_never_break_mid_item() {
        let items = environment_items(
            Some("Berlin"),
            Some(&weather(Some("clear"), Some(21.0), None)),
            None,
            Some(&air(Some(72))),
        );
        // "o 21°C clear" (12) + " * " + "! AQI 72" (8) + " * " + "@ Berlin" (8)
        let wide = env_strip_rows(40, &items);
        assert_eq!(wide.len(), 1);
        assert_eq!(wide[0].len(), 3);
        assert_eq!(env_strip_height(40, &items), 1);

        // At 25 cells the location no longer fits the first row; the AQI item
        // moves whole, never split.
        let narrow = env_strip_rows(25, &items);
        assert_eq!(narrow.len(), 2);
        assert_eq!(texts(&narrow[0]), ["21°C clear", "AQI 72"]);
        assert_eq!(texts(&narrow[1]), ["Berlin"]);
        assert_eq!(env_strip_height(25, &items), 2);
    }

    #[test]
    fn an_oversize_item_wraps_hanging_under_its_glyph() {
        let items = environment_items(
            Some("Mountain trail near Boulder, Colorado, two hours north of the city"),
            None,
            None,
            None,
        );
        let rows = env_strip_rows(24, &items);
        assert!(rows.len() > 1);
        assert_eq!(rows[0][0].glyph, Some('@'));
        assert!(!rows[0][0].continuation);
        for row in &rows[1..] {
            assert_eq!(row[0].glyph, None);
            assert!(row[0].continuation, "wrapped rows must indent, not lead");
        }
        // Every produced row still fits the strip.
        for row in &rows {
            let width: u16 = row.iter().map(EnvItem::width).sum::<u16>()
                + SEPARATOR_WIDTH * (row.len() as u16 - 1);
            assert!(width <= 24, "row overflows: {row:?}");
        }
    }
}
