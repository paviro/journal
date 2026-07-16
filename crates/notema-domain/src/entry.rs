use crate::Coordinates;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Deserializer, Serialize};
use std::ops::RangeInclusive;
use std::path::PathBuf;

/// The supported mood range. Out-of-range values are dropped to `None` on read
/// (see [`Metadata`]) and rejected at the CLI boundary.
pub const MOOD_RANGE: RangeInclusive<i8> = -5..=5;

/// The user-assignable metadata carried by the create/import/edit paths: the
/// free-text lists, an optional mood, the starred flag, and where the entry was
/// written. Not a field on [`Entry`] (whose front-matter fields are flat); it
/// serves as the construction bundle and the codec's flatten carrier.
///
/// Not `Eq`: `location` carries `f64` coordinates.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feelings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_mood",
        skip_serializing_if = "Option::is_none"
    )]
    pub mood: Option<i8>,
    /// Whether the user flagged this entry as a favorite. Omitted from front
    /// matter when false so existing files stay byte-stable.
    #[serde(default, skip_serializing_if = "is_unstarred")]
    pub starred: bool,
    /// Where the entry was written. Skipped by serde here: the `[location]` TOML
    /// table is owned by the front-matter codec (so the flattened scalars stay
    /// byte-stable), which syncs it to/from this field at the boundary.
    #[serde(skip)]
    pub location: Option<Location>,
}

fn is_unstarred(starred: &bool) -> bool {
    !*starred
}

impl Metadata {
    /// The location rendered as its one-line label, if a location with named
    /// parts or coordinates is set. The entry-view metadata section shows this.
    pub fn location_label(&self) -> Option<String> {
        self.location
            .as_ref()
            .and_then(|location| location.display_label())
    }
}

/// Where an entry was written — coordinates plus a fine-to-coarse place
/// hierarchy. Each field is one OpenStreetMap / Nominatim address key, stored 1:1
/// with no collapsing; whatever a geocode returns is kept and the rest omitted.
/// First captured on import (Day One), now also user-editable via the location
/// dialog. An all-empty location is dropped entirely.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Location {
    /// A human label for the place — a POI/venue name from the geocoder, or one
    /// the user typed. (Nominatim's top-level `name`, not an address key.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub house_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub road: Option<String>,
    // City subdivisions, finest to coarsest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neighbourhood: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suburb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub borough: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city_district: Option<String>,
    // Settlement — Nominatim returns exactly one of these by size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub town: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub village: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub municipality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hamlet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postcode: Option<String>,
    // Region within the country, finest to coarsest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub county: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_district: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub province: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    /// Horizontal accuracy in metres, set only for a device ("grab GPS") fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy_m: Option<f64>,
    /// The provider a device fix came from (e.g. `corelocation`); unset otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Provenance of an imported entry: which tool it came from and that tool's own
/// identifier for it (e.g. `source = "dayone"`, `id = "<UUID>"`). Serialized as
/// the `[import]` front-matter table; absent for entries created in the app. The
/// (source, id) pair is what importers dedup on to skip re-importing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ImportSource {
    pub source: String,
    pub id: String,
}

impl Location {
    pub fn coordinates(&self) -> Option<Coordinates> {
        Coordinates::try_new(self.latitude?, self.longitude?).ok()
    }

    pub fn set_coordinates(&mut self, coordinates: Coordinates) {
        self.latitude = Some(coordinates.latitude());
        self.longitude = Some(coordinates.longitude());
    }

    pub fn is_empty(&self) -> bool {
        *self == Location::default()
    }

    /// Whether any named part is set — i.e. more than the bare coordinates (and
    /// the device-fix metadata that rides along with them). Used to tell a
    /// fully-resolved location from one that only has a lat/lon pair still
    /// awaiting a reverse lookup.
    pub fn has_named_parts(&self) -> bool {
        let coords_only = Location {
            latitude: self.latitude,
            longitude: self.longitude,
            accuracy_m: self.accuracy_m,
            source: self.source.clone(),
            ..Location::default()
        };
        *self != coords_only
    }

    /// Overlay these names onto `pin`'s coordinates: keep `pin`'s
    /// latitude/longitude/accuracy/source (a reverse lookup must never move the
    /// pin) and take every named part from `self`.
    pub fn with_pin_from(mut self, pin: &Location) -> Location {
        self.latitude = pin.latitude.or(self.latitude);
        self.longitude = pin.longitude.or(self.longitude);
        self.accuracy_m = pin.accuracy_m.or(self.accuracy_m);
        self.source = pin.source.clone().or(self.source);
        self
    }

    /// The settlement name, whichever size key Nominatim used (city → hamlet).
    fn settlement(&self) -> Option<&str> {
        self.city
            .as_deref()
            .or(self.town.as_deref())
            .or(self.village.as_deref())
            .or(self.municipality.as_deref())
            .or(self.hamlet.as_deref())
    }

    /// A one-line label for display. The `name` is set off with `" - "`; the
    /// address parts follow, joined by `", "`. Only a curated subset is shown —
    /// road + house number, `neighbourhood`, `suburb`, postcode + settlement, and
    /// `country`; the coarser subdivisions (`quarter`, `borough`, `city_district`)
    /// and the region hierarchy (`county`/`state`/…) are stored but omitted to keep
    /// the label readable. Falls back to the coordinates when no names are known;
    /// `None` when nothing at all is known.
    pub fn display_label(&self) -> Option<String> {
        let street_line = match (self.road.as_deref(), self.house_number.as_deref()) {
            (Some(road), Some(number)) => Some(format!("{road} {number}")),
            (Some(road), None) => Some(road.to_string()),
            // A bare house number without a road is meaningless on its own.
            (None, _) => None,
        };
        let settlement_line = match (self.postcode.as_deref(), self.settlement()) {
            (Some(postcode), Some(settlement)) => Some(format!("{postcode} {settlement}")),
            (None, Some(settlement)) => Some(settlement.to_string()),
            (Some(postcode), None) => Some(postcode.to_string()),
            (None, None) => None,
        };
        let address: Vec<String> = [
            street_line,
            self.neighbourhood.clone(),
            self.suburb.clone(),
            settlement_line,
            self.country.clone(),
        ]
        .into_iter()
        .flatten()
        .collect();

        match (&self.name, address.is_empty()) {
            (Some(name), true) => Some(name.clone()),
            (Some(name), false) => Some(format!("{name} - {}", address.join(", "))),
            (None, false) => Some(address.join(", ")),
            (None, true) => match (self.latitude, self.longitude) {
                (Some(lat), Some(lon)) => Some(format!("{lat:.4}, {lon:.4}")),
                _ => None,
            },
        }
    }
}

/// The `[weather]` table. `condition` is a condition slug (e.g.
/// `"partly-cloudy"`). `source` names the provider the data came from, kept for
/// attribution. Every field is optional — only what the source provided is
/// stored. First captured on Day One import; now also fetched from Open-Meteo
/// after a location is set.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Weather {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_celsius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feels_like_celsius: Option<f64>,
    /// The dew point — the temperature at which the air would saturate; a truer
    /// "mugginess" signal than relative humidity alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dew_point_celsius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humidity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure_mb: Option<f64>,
    /// Total sky cloudiness, as a 0–1 fraction (like `humidity`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_cover: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_km: Option<f64>,
    /// Precipitation total for the hour, in millimetres (rain plus melted snow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precipitation_mm: Option<f64>,
    /// The sustained wind speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind_speed_kph: Option<f64>,
    /// The peak momentary gust (always ≥ `wind_speed_kph`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind_gust_kph: Option<f64>,
    /// The direction the wind blows *from*, as a compass bearing in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind_direction: Option<f64>,
    /// The weather data provider/service, stored verbatim for attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// The `[celestial]` table: sun/moon at the time of writing. `moon_phase` is the
/// 0–1 cycle fraction; `moon_phase_name` its named phase; `sunrise`/`sunset` are
/// RFC3339 timestamps; `day_length_seconds` is the daylight duration between
/// them. First captured on Day One import; now also computed locally from the
/// location's coordinates and date.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Celestial {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moon_phase: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moon_phase_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sunrise: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sunset: Option<String>,
    /// Daylight duration in seconds — the sun-above-horizon span (sunset −
    /// sunrise), excluding twilight. `None` when either event is absent (polar
    /// day/night), mirroring `sunrise`/`sunset`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_length_seconds: Option<u64>,
}

/// The `[air_quality]` table: pollution and UV at the time of writing. Its own
/// table rather than part of `[weather]` — it comes from a separate provider
/// endpoint and is written independently, so an entry may carry one without the
/// other. `source` names the provider, kept for attribution. Every field is
/// optional; only what the source provided is stored.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct AirQuality {
    /// European Air Quality Index (0–100+, lower is better).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub european_aqi: Option<i64>,
    /// United States Air Quality Index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub us_aqi: Option<i64>,
    /// Fine particulate matter (≤2.5µm), in µg/m³.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pm2_5: Option<f64>,
    /// Coarse particulate matter (≤10µm), in µg/m³.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pm10: Option<f64>,
    /// Carbon monoxide, in µg/m³.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbon_monoxide: Option<f64>,
    /// Nitrogen dioxide, in µg/m³.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nitrogen_dioxide: Option<f64>,
    /// Ground-level ozone, in µg/m³.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ozone: Option<f64>,
    /// Sulphur dioxide, in µg/m³.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sulphur_dioxide: Option<f64>,
    /// The UV index — served by this same air-quality endpoint, not the weather
    /// one, so it is stored here rather than in `[weather]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv_index: Option<f64>,
    /// Birch pollen, in grains/m³ (Europe only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birch_pollen: Option<f64>,
    /// Grass pollen, in grains/m³ (Europe only; `None` elsewhere).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grass_pollen: Option<f64>,
    /// Ragweed pollen, in grains/m³ (Europe only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ragweed_pollen: Option<f64>,
    /// The provider/service, stored verbatim for attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl Weather {
    /// Whether no field carries data — every field is `Option`, so an all-`None`
    /// value equals the default.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl Celestial {
    /// Whether no field carries data (see [`Weather::is_empty`]).
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl AirQuality {
    /// Whether no field carries data (see [`Weather::is_empty`]).
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Read `mood` as an integer and clamp it to [`MOOD_RANGE`], dropping
/// out-of-range values to `None` without failing the whole parse.
fn deserialize_mood<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i8>, D::Error> {
    let raw = Option::<i64>::deserialize(deserializer)?;
    Ok(raw
        .and_then(|value| i8::try_from(value).ok())
        .filter(|value| MOOD_RANGE.contains(value)))
}

/// An entry's creation time in both forms it is needed: the exact RFC3339
/// string as written on disk (round-trip fidelity, e.g. for imports) and the
/// value parsed once at load, so the grouping, label, and stats paths never
/// re-run `DateTime::parse_from_rfc3339` per call. `parsed` is `None` when the
/// string is not valid RFC3339 (callers then fall back to the filename date).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timestamp {
    pub raw: String,
    /// The parsed value keeps the RFC3339 offset it was written with rather than
    /// normalizing to the machine's local zone, so an entry always renders at the
    /// wall-clock time it was written in — regardless of where it is now read.
    pub parsed: Option<DateTime<FixedOffset>>,
}

impl Timestamp {
    /// Parse `raw` as RFC3339 once, keeping the original string regardless.
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let parsed = DateTime::parse_from_rfc3339(&raw).ok();
        Self { raw, parsed }
    }
}

// Not `Eq`: `Location` carries `f64` coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub journal: String,
    pub path: PathBuf,
    pub encryption_state: EntryEncryptionState,
    pub created_at: Option<Timestamp>,
    pub edited_at: Option<String>,
    pub preview: String,
    // Front-matter metadata, one flat field each (see [`Metadata`] for the
    // construction bundle that mirrors the editable subset).
    pub activities: Vec<String>,
    pub feelings: Vec<String>,
    pub people: Vec<String>,
    pub tags: Vec<String>,
    pub mood: Option<i8>,
    pub starred: bool,
    /// Where the entry was written.
    pub location: Option<Location>,
    /// Captured weather from front matter, when present.
    pub weather: Option<Weather>,
    /// Captured sun/moon data from front matter, when present.
    pub celestial: Option<Celestial>,
    /// Captured air quality from front matter, when present.
    pub air_quality: Option<AirQuality>,
    /// Provenance of an imported entry (source tool + its id). `None` for
    /// entries created directly in the app. Used to skip re-importing and as an
    /// anchor for back-filling richer metadata once the format supports it.
    pub import: Option<ImportSource>,
    pub body: String,
    /// Word count of `body`, computed once at load so the entry-list row
    /// builder never tokenizes the full body on the render path.
    pub word_count: usize,
    /// `body` plus every metadata value merged into one normalized (lowercased,
    /// accent-folded) string, built once at load ([`build_search_haystack`]) so
    /// whole-corpus word search never rebuilds or re-normalizes the haystack per
    /// entry per keystroke.
    pub search_haystack: String,
    /// A non-fatal load problem. The body remains readable, but metadata edits
    /// are blocked until the front matter is repaired.
    pub warning: Option<String>,
}

impl Entry {
    /// The raw RFC3339 creation string as written on disk, if any.
    pub fn created_raw(&self) -> Option<&str> {
        self.created_at
            .as_ref()
            .map(|timestamp| timestamp.raw.as_str())
    }

    /// The creation timestamp parsed once at load, if present and well-formed.
    pub fn created_time(&self) -> Option<DateTime<FixedOffset>> {
        self.created_at
            .as_ref()
            .and_then(|timestamp| timestamp.parsed)
    }

    /// The entry's metadata cloned into a [`Metadata`] bundle — the shape the
    /// editor buffers and the entry-view metadata section renders from.
    pub fn metadata_bundle(&self) -> Metadata {
        Metadata {
            activities: self.activities.clone(),
            feelings: self.feelings.clone(),
            people: self.people.clone(),
            tags: self.tags.clone(),
            mood: self.mood,
            starred: self.starred,
            location: self.location.clone(),
        }
    }

    /// A non-empty label for the entry: the start of the preview, else the
    /// created timestamp, else the id.
    pub fn display_label(&self) -> String {
        let preview = self.preview.trim();
        if !preview.is_empty() {
            return preview.chars().take(80).collect();
        }
        self.created_raw()
            .map(str::to_string)
            .unwrap_or_else(|| self.id.clone())
    }
}

/// Merge the body and every metadata value into one space-separated string, the
/// haystack a prefix-less word query is matched against. Normalized (lowercased,
/// accent-folded) so search never has to re-normalize it. Precomputed at load
/// into [`Entry::search_haystack`].
pub fn build_search_haystack(content: &str, metadata: &Metadata) -> String {
    let mut buf = String::with_capacity(content.len() + 16);
    push_normalized(content, &mut buf);
    for value in metadata
        .activities
        .iter()
        .chain(&metadata.feelings)
        .chain(&metadata.people)
        .chain(&metadata.tags)
    {
        buf.push(' ');
        push_normalized(value, &mut buf);
    }
    buf
}

/// Lowercase and accent-fold `s`, returning a fresh `String`. Used to normalize
/// a search query so it matches the pre-normalized [`Entry::search_haystack`].
pub fn normalize_for_search(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    push_normalized(s, &mut out);
    out
}

/// Append `s`, lowercased and accent-folded, to `out`. ASCII takes a branch-only
/// fast path (the overwhelming majority of text); only non-ASCII chars consult
/// the diacritic table.
fn push_normalized(s: &str, out: &mut String) {
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
        } else {
            for lc in c.to_lowercase() {
                match fold_diacritic(lc) {
                    Some(base) => out.push_str(base),
                    None => out.push(lc),
                }
            }
        }
    }
}

/// Map a lowercased Latin accented letter to its ASCII base, so `café`↔`cafe`,
/// `über`↔`uber`, and `straße`↔`strasse` match. Chars absent from the table
/// (incl. non-Latin scripts) are matched unchanged.
fn fold_diacritic(c: char) -> Option<&'static str> {
    Some(match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'č' => "c",
        'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
        'í' | 'ì' | 'î' | 'ï' | 'ī' => "i",
        'ñ' | 'ń' => "n",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'ō' => "o",
        'œ' => "oe",
        'ß' => "ss",
        'ú' | 'ù' | 'û' | 'ü' | 'ū' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'ś' | 'š' => "s",
        'ł' => "l",
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryEncryptionState {
    Plain,
    EncryptedUnlocked,
    EncryptedLocked,
    EncryptedUnreadable,
}

/// One front-matter metadata field paired with its new value, for targeted
/// single-field edits (see `set_entry_metadata_field`).
// Not `Eq`: the `Location` payload carries `f64` coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum MetadataField {
    Tags(Vec<String>),
    People(Vec<String>),
    Activities(Vec<String>),
    Feelings(Vec<String>),
    Mood(Option<i8>),
    Starred(bool),
    /// The whole `[location]` table. `None` clears it. Boxed because `Location`
    /// is far larger than the other variants.
    Location(Option<Box<Location>>),
    /// The whole `[weather]` table. `None` clears it. Written independently of
    /// `[celestial]` so the network-fetched weather and the locally-computed
    /// celestial data land as separate writes. Boxed to keep the enum small.
    Weather(Option<Box<Weather>>),
    /// The whole `[celestial]` table. `None` clears it. Boxed to match `Weather`.
    Celestial(Option<Box<Celestial>>),
    /// The whole `[air_quality]` table. `None` clears it. Fetched from a separate
    /// endpoint than `[weather]`, so it lands as its own write. Boxed to match.
    AirQuality(Option<Box<AirQuality>>),
}

pub struct EntryPath {
    pub journal: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub id: String,
    pub journal: String,
    pub created_at: Option<String>,
    pub title: String,
    pub preview: String,
    pub starred: bool,
}

impl SearchHit {
    pub fn from_entry(entry: &Entry) -> Self {
        Self {
            id: entry.id.clone(),
            journal: entry.journal.clone(),
            created_at: entry.created_raw().map(str::to_string),
            title: entry.display_label(),
            preview: entry.preview.clone(),
            starred: entry.starred,
        }
    }
}

/// Which journals a search covers. Owned so the same value serves as UI state
/// and as the argument borrowed by the application's search engine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SearchScope {
    #[default]
    AllJournals,
    Journal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_display_label_sets_off_name_and_omits_coarse_fields() {
        let location = Location {
            name: Some("Zuhause".to_string()),
            road: Some("Gürtelstraße".to_string()),
            house_number: Some("13".to_string()),
            neighbourhood: Some("Komponistenviertel".to_string()),
            suburb: Some("Weißensee".to_string()),
            city_district: Some("Pankow".to_string()),
            postcode: Some("13088".to_string()),
            city: Some("Berlin".to_string()),
            state: Some("Berlin".to_string()),
            country: Some("Deutschland".to_string()),
            latitude: Some(52.5449),
            longitude: Some(13.4532),
            ..Location::default()
        };
        // `name` is set off with " - "; `city_district` (Pankow) and `state` are
        // stored but not shown.
        assert_eq!(
            location.display_label().as_deref(),
            Some(
                "Zuhause - Gürtelstraße 13, Komponistenviertel, Weißensee, 13088 Berlin, Deutschland"
            )
        );
    }

    #[test]
    fn location_display_label_falls_back_to_coordinates() {
        let location = Location {
            latitude: Some(10.0),
            longitude: Some(20.0),
            ..Location::default()
        };
        assert_eq!(
            location.display_label().as_deref(),
            Some("10.0000, 20.0000")
        );
        assert_eq!(Location::default().display_label(), None);
    }
}
