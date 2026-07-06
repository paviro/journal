//! Serde model for a Day One JSON export.
//!
//! The export is a `metadata` block plus an `entries` array. Media files
//! (photos/audios/videos/pdfs) live in sibling folders next to the JSON and are
//! named on disk by their `md5` (e.g. `photos/<md5>.<type>`), while the entry
//! body references them by `identifier` via `dayone-moment://<identifier>`.
//!
//! Many fields here are parsed but not yet consumed by the importer: location,
//! weather, starred/pinned flags, per-entry timezone, and the non-photo media
//! arrays. They are modeled deliberately so the import is *ready* to map them
//! once the journal format grows matching fields — hence the module-wide
//! `dead_code` allowance.
#![allow(dead_code)]

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct DayOneExport {
    #[serde(default)]
    pub entries: Vec<DayOneEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayOneEntry {
    pub uuid: String,
    /// Markdown body. Day One escapes literal punctuation with backslashes.
    #[serde(default)]
    pub text: Option<String>,
    /// Clean structured body, present on newer entries. A JSON string (Day One's
    /// `ZRICHTEXTJSON`) that renders to faithful Markdown — preferred over `text`
    /// when available. See [`crate::dayone::richtext`].
    #[serde(default)]
    pub rich_text: Option<String>,
    /// RFC3339 (UTC) creation timestamp.
    pub creation_date: Option<String>,
    /// RFC3339 (UTC) last-modified timestamp.
    pub modified_date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,

    // --- Parsed but not imported yet (ready for future mapping) ---
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_all_day: bool,
    pub time_zone: Option<String>,
    pub location: Option<Location>,
    pub weather: Option<Weather>,

    // Media. Only `photos` are imported today; the rest are modeled with the
    // same fidelity so they can be ingested once the asset system supports
    // non-image files.
    #[serde(default)]
    pub photos: Vec<Moment>,
    #[serde(default)]
    pub audios: Vec<Moment>,
    #[serde(default)]
    pub videos: Vec<Moment>,
    #[serde(default)]
    pub pdf_attachments: Vec<Moment>,
}

/// A media attachment ("moment"). Referenced in the body by `identifier`, stored
/// on disk as `<folder>/<md5>.<type>`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub identifier: String,
    pub md5: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub favorite: Option<bool>,
    pub date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub place_name: Option<String>,
    pub locality_name: Option<String>,
    pub administrative_area: Option<String>,
    pub country: Option<String>,
    pub time_zone_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Weather {
    pub conditions_description: Option<String>,
    pub temperature_celsius: Option<f64>,
    pub weather_code: Option<String>,
    pub relative_humidity: Option<f64>,
    pub wind_speed_kph: Option<f64>,
}
