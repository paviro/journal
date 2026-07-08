use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Deserializer, Serialize};
use std::ops::RangeInclusive;
use std::path::PathBuf;

/// The supported mood range. Out-of-range values are dropped to `None` on read
/// (see [`Metadata`]) and rejected at the CLI boundary.
pub const MOOD_RANGE: RangeInclusive<i8> = -5..=5;

/// The user-assignable metadata shared by an [`Entry`], its on-disk front
/// matter, and the create/import paths: free-text tag lists plus an optional
/// mood score. One shape, defined once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feelings: Vec<String>,
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
}

fn is_unstarred(starred: &bool) -> bool {
    !*starred
}

/// Where an entry was written, captured on import (Day One) — a coarse-to-fine
/// place hierarchy plus coordinates. Every field is optional: only what the
/// source provided is stored, and an all-empty location is dropped entirely.
/// Capture-only: displayed but not edited or searched.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Location {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub administrative_area: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
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
    pub fn is_empty(&self) -> bool {
        self.place.is_none()
            && self.locality.is_none()
            && self.administrative_area.is_none()
            && self.country.is_none()
            && self.latitude.is_none()
            && self.longitude.is_none()
    }

    /// A one-line label: the named parts (coarse-to-fine) joined by `", "`, or
    /// the coordinates when no names are known. `None` when nothing is known.
    pub fn display_label(&self) -> Option<String> {
        let parts: Vec<&str> = [
            self.place.as_deref(),
            self.locality.as_deref(),
            self.administrative_area.as_deref(),
            self.country.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !parts.is_empty() {
            return Some(parts.join(", "));
        }
        match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) => Some(format!("{lat:.4}, {lon:.4}")),
            _ => None,
        }
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub journal: String,
    pub path: PathBuf,
    pub encryption_state: EntryEncryptionState,
    pub created_at: Option<Timestamp>,
    pub edited_at: Option<String>,
    pub preview: String,
    pub metadata: Metadata,
    /// Where the entry was written, captured on import. Displayed but not edited
    /// or searched, so it lives outside [`Metadata`].
    pub location: Option<Location>,
    /// Provenance of an imported entry (source tool + its id). `None` for
    /// entries created directly in the app. Used to skip re-importing and as an
    /// anchor for back-filling richer metadata once the format supports it.
    pub import: Option<ImportSource>,
    pub content: String,
    /// Word count of `content`, computed once at load so the entry-list row
    /// builder never tokenizes the full body on the render path.
    pub word_count: usize,
    /// `content` plus every metadata value merged into one string, built once at
    /// load ([`build_search_haystack`]) so whole-corpus fuzzy search never
    /// rebuilds the haystack per entry per keystroke.
    pub search_haystack: String,
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
/// haystack a prefix-less fuzzy query is scored against. Precomputed at load into
/// [`Entry::search_haystack`].
pub fn build_search_haystack(content: &str, metadata: &Metadata) -> String {
    let mut buf = String::with_capacity(content.len() + 16);
    buf.push_str(content);
    for value in metadata
        .tags
        .iter()
        .chain(&metadata.people)
        .chain(&metadata.activities)
        .chain(&metadata.feelings)
    {
        buf.push(' ');
        buf.push_str(value);
    }
    buf
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryEncryptionState {
    Plain,
    EncryptedUnlocked,
    EncryptedLocked,
}

/// One front-matter metadata field paired with its new value, for targeted
/// single-field edits (see `set_entry_metadata_field`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataField {
    Tags(Vec<String>),
    People(Vec<String>),
    Activities(Vec<String>),
    Feelings(Vec<String>),
    Mood(Option<i8>),
    Starred(bool),
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
            starred: entry.metadata.starred,
        }
    }
}

/// Which journals a search covers. Owned so the same value serves as UI state
/// and as the argument borrowed into [`search_loaded_entries`](crate::search_loaded_entries).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SearchScope {
    #[default]
    AllJournals,
    Journal(String),
}
