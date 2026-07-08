use journal_core::{ImportSource, Location, Metadata, MetadataField};
use serde::{Deserialize, Serialize};

/// Every entry front-matter field, parsed and serialized in a single TOML pass.
/// The user metadata is the shared [`Metadata`] type, flattened so its fields
/// sit at the top level of the front matter (mood clamped on read there). The
/// system/provenance fields group into TOML tables, which — being tables — must
/// all follow the flattened scalars: hence `metadata` comes first.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct FrontMatter {
    #[serde(flatten)]
    pub metadata: Metadata,
    #[serde(default, skip_serializing_if = "Datetime::is_empty")]
    pub datetime: Datetime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportSource>,
    /// Where the entry was written (Day One import).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// Weather at the time of writing (Day One import). A TOML table with a
    /// nested `[weather.wind]`, so it stays among the trailing tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weather: Option<Weather>,
    /// Sun/moon at the time of writing (Day One import) — astronomy, kept as its
    /// own table rather than under weather.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub celestial: Option<Celestial>,
}

/// The `[datetime]` table: when an entry was created and last edited, the IANA
/// zone it was authored in, and how long was spent editing it. `timezone` is
/// capture-only — the offset already lives in `created_at`, but the zone *name* it
/// can't recover, so we keep it. `writing_seconds` accumulates the editor-open
/// time across edits (seeded from Day One's `editingTime` on import).
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Datetime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writing_seconds: Option<u64>,
}

impl Datetime {
    fn is_empty(&self) -> bool {
        self.created_at.is_none()
            && self.edited_at.is_none()
            && self.timezone.is_none()
            && self.writing_seconds.is_none()
    }
}

/// The `[weather]` table (Day One import, capture-only). `condition` is Day One's
/// condition slug (e.g. `"partly-cloudy"`). Every field is optional — only what
/// the export provided is stored.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Weather {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_celsius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feels_like_celsius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humidity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure_mb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_km: Option<f64>,
    // A TOML sub-table, so it must come after weather's scalar fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind: Option<Wind>,
}

/// The `[weather.wind]` sub-table. `direction` is a compass bearing in degrees.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Wind {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_kph: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<f64>,
}

/// The `[celestial]` table (Day One import, capture-only): sun/moon at the time
/// of writing. `moon_phase` is the 0–1 cycle fraction; `moon_phase_name` its
/// named phase; `sunrise`/`sunset` are RFC3339 timestamps.
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
}

impl Weather {
    pub fn is_empty(&self) -> bool {
        self.condition.is_none()
            && self.temperature_celsius.is_none()
            && self.feels_like_celsius.is_none()
            && self.humidity.is_none()
            && self.pressure_mb.is_none()
            && self.visibility_km.is_none()
            && self.wind.is_none()
    }
}

impl Wind {
    pub fn is_empty(&self) -> bool {
        self.speed_kph.is_none() && self.direction.is_none()
    }
}

impl Celestial {
    pub fn is_empty(&self) -> bool {
        self.moon_phase.is_none()
            && self.moon_phase_name.is_none()
            && self.sunrise.is_none()
            && self.sunset.is_none()
    }
}

pub fn split_front_matter(content: &str) -> (Option<&str>, &str) {
    let Some(rest) = content.strip_prefix("+++\n") else {
        return (None, content);
    };

    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let marker = line.trim_end_matches('\n').trim_end_matches('\r');
        if marker == "+++" {
            let front_matter = rest[..offset].trim_end_matches('\n').trim_end_matches('\r');
            let body = &rest[offset + line.len()..];
            return (Some(front_matter), body);
        }
        offset += line.len();
    }

    if let Some(index) = rest.rfind('\n') {
        let marker = &rest[index + 1..];
        if marker == "+++" {
            let front_matter = rest[..index].trim_end_matches('\r');
            return (Some(front_matter), "");
        }
    }

    (None, content)
}

/// Parse every front-matter field at once. Malformed TOML yields defaults.
pub fn front_matter_fields(front_matter: &str) -> FrontMatter {
    parse_front_matter(front_matter).unwrap_or_default()
}

/// A one-line summary of the body: display lines collapsed onto a single line,
/// with markdown markers stripped and space-wasting constructs redacted to short
/// placeholders (fenced code -> `[code]`, images -> `[image]`, links -> `[link]`).
pub fn display_preview(body: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut in_code = false;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if !in_code {
                parts.push("[code]".to_string());
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if let Some(text) = display_line_text(line) {
            parts.push(redact_inline(text));
        }
    }

    truncate_preview(&parts.join(" "))
}

/// Replace markdown images (`![alt](url)`) with `[image]` and links
/// (`[text](url)`) with `[link]` so their URLs don't waste preview space.
fn redact_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(bracket) = rest.find('[') {
        let is_image = bracket > 0 && rest.as_bytes()[bracket - 1] == b'!';
        let marker = if is_image { bracket - 1 } else { bracket };
        if let Some(span) = journal_core::markdown::parse_inline_at(&rest[marker..]) {
            out.push_str(&rest[..marker]);
            out.push_str(if span.is_image { "[image]" } else { "[link]" });
            rest = &rest[marker + span.span.end..];
        } else {
            out.push_str(&rest[..bracket + 1]);
            rest = &rest[bracket + 1..];
        }
    }
    out.push_str(rest);

    out
}

/// Parse the front matter, apply `mutate`, and re-render the whole file.
/// Returns `None` when there is no front matter or it fails to parse.
fn map_front_matter(content: &str, mutate: impl FnOnce(&mut FrontMatter)) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let mut metadata = parse_front_matter(front_matter?)?;
    mutate(&mut metadata);
    Some(render_entry(&metadata, body))
}

/// Return a copy of `content` with one metadata field replaced in the front
/// matter and `edited_at` refreshed. `None` when there is no front matter.
pub fn with_metadata_field(content: &str, field: &MetadataField) -> Option<String> {
    map_front_matter(content, |fm| {
        match field {
            MetadataField::Tags(values) => fm.metadata.tags = values.clone(),
            MetadataField::People(values) => fm.metadata.people = values.clone(),
            MetadataField::Activities(values) => fm.metadata.activities = values.clone(),
            MetadataField::Feelings(values) => fm.metadata.feelings = values.clone(),
            MetadataField::Mood(mood) => fm.metadata.mood = *mood,
            MetadataField::Starred(starred) => fm.metadata.starred = *starred,
        }
        fm.datetime.edited_at = Some(chrono::Local::now().to_rfc3339());
    })
}

/// Return a copy of `content` with `secs` added to `[datetime].writing_seconds`.
/// Unlike [`with_metadata_field`], it does **not** refresh `edited_at` — recording
/// editing time is not itself a content edit. `None` when there is no front matter.
pub(crate) fn add_writing_seconds(content: &str, secs: u64) -> Option<String> {
    map_front_matter(content, |fm| {
        let total = fm
            .datetime
            .writing_seconds
            .unwrap_or(0)
            .saturating_add(secs);
        fm.datetime.writing_seconds = Some(total);
    })
}

pub(crate) fn parse_front_matter(front_matter: &str) -> Option<FrontMatter> {
    toml::from_str(front_matter).ok()
}

/// Render an entry from its front matter and body: the one canonical framing
/// used by create, edit, asset-rewrite, and metadata edits. Leading blank lines
/// of `body` are dropped so a single blank line always separates the fence from
/// the body.
pub(crate) fn render_entry(front_matter: &FrontMatter, body: &str) -> String {
    let toml = toml::to_string(front_matter).unwrap_or_default();
    format!("+++\n{toml}+++\n\n{}", body.trim_start_matches('\n'))
}

fn display_line_text(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_heading = markdown_heading_text(trimmed).unwrap_or(trimmed);
    if without_heading.is_empty() {
        None
    } else {
        Some(without_heading)
    }
}

fn markdown_heading_text(line: &str) -> Option<&str> {
    if !line.starts_with('#') {
        return None;
    }

    let after_hashes = line.trim_start_matches('#');
    if after_hashes.starts_with(' ') {
        Some(after_hashes.trim())
    } else {
        None
    }
}

fn truncate_preview(line: &str) -> String {
    line.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_preview_collapses_body_with_markdown_stripped() {
        assert_eq!(
            display_preview("## Real Title\nBody text\nMore body"),
            "Real Title Body text More body"
        );
    }

    #[test]
    fn display_preview_is_empty_when_body_blank() {
        assert_eq!(display_preview("\n\n"), "");
    }

    #[test]
    fn display_preview_redacts_fenced_code_blocks() {
        let body = "Before\n```rust\nfn main() {}\nlet x = 1;\n```\nAfter";
        assert_eq!(display_preview(body), "Before [code] After");
    }

    #[test]
    fn display_preview_redacts_images_and_links() {
        let body = "See ![a cat](cat.png) and [the docs](https://example.com/x) here";
        assert_eq!(display_preview(body), "See [image] and [link] here");
    }

    #[test]
    fn split_front_matter_parses_toml_delimiters() {
        let (front_matter, body) = split_front_matter("+++\ntitle = \"A\"\n+++\n\n# Body\n");

        assert_eq!(front_matter, Some("title = \"A\""));
        assert_eq!(body, "\n# Body\n");
    }

    #[test]
    fn front_matter_tags_reads_list() {
        let tags = front_matter_fields("tags = [\"foo\", \"bar\"]\n")
            .metadata
            .tags;

        assert_eq!(tags, vec!["foo", "bar"]);
    }

    #[test]
    fn front_matter_tags_handles_commas_in_values() {
        let tags = front_matter_fields("tags = [\"foo, bar\", \"baz\"]\n")
            .metadata
            .tags;

        assert_eq!(tags, vec!["foo, bar", "baz"]);
    }

    #[test]
    fn front_matter_feelings_reads_list() {
        let feelings = front_matter_fields("feelings = [\"calm\", \"focused\"]\n")
            .metadata
            .feelings;

        assert_eq!(feelings, vec!["calm", "focused"]);
    }

    #[test]
    fn mood_is_clamped_to_supported_range() {
        assert_eq!(front_matter_fields("mood = 3\n").metadata.mood, Some(3));
        assert_eq!(front_matter_fields("mood = -5\n").metadata.mood, Some(-5));
        assert_eq!(front_matter_fields("mood = 5\n").metadata.mood, Some(5));
        // Out of range or non-integer moods drop to None rather than failing.
        assert_eq!(front_matter_fields("mood = 6\n").metadata.mood, None);
        assert_eq!(front_matter_fields("mood = -42\n").metadata.mood, None);
        assert_eq!(front_matter_fields("mood = 999\n").metadata.mood, None);
    }

    #[test]
    fn with_metadata_field_writes_and_clears_mood() {
        let content = "+++\n[datetime]\ncreated_at = \"x\"\n+++\n\n# Body\n";

        let with_mood = with_metadata_field(content, &MetadataField::Mood(Some(4))).unwrap();
        assert_eq!(
            front_matter_fields(split_front_matter(&with_mood).0.unwrap())
                .metadata
                .mood,
            Some(4)
        );

        let cleared = with_metadata_field(&with_mood, &MetadataField::Mood(None)).unwrap();
        assert_eq!(
            front_matter_fields(split_front_matter(&cleared).0.unwrap())
                .metadata
                .mood,
            None
        );
    }

    #[test]
    fn tables_serialize_after_flattened_scalars_and_round_trip() {
        // The gate: the `[datetime]`, `[import]`, and `[location]` tables must
        // serialize *after* the flattened `metadata` scalars (TOML requires
        // tables last) and re-parse. render_entry swallows a serialize failure
        // into empty output, so assert the tables are actually there.
        let fm = FrontMatter {
            metadata: Metadata {
                tags: vec!["dream".to_string()],
                ..Metadata::default()
            },
            datetime: Datetime {
                created_at: Some("2021-04-03T08:30:05+02:00".to_string()),
                timezone: Some("Europe/Berlin".to_string()),
                ..Datetime::default()
            },
            import: Some(ImportSource {
                source: "dayone".to_string(),
                id: "X".to_string(),
            }),
            location: Some(Location {
                place: Some("1 Example Plaza".to_string()),
                locality: Some("Testville".to_string()),
                country: Some("Testland".to_string()),
                latitude: Some(10.0),
                ..Location::default()
            }),
            ..FrontMatter::default()
        };

        let rendered = render_entry(&fm, "# Body\n");
        assert!(rendered.contains("[location]"), "table missing: {rendered}");
        // The flattened `tags` scalar precedes every table (valid TOML), and the
        // tables keep struct order: [datetime], [import], [location].
        let tags_at = rendered.find("tags = ").unwrap();
        let dates_at = rendered.find("[datetime]").unwrap();
        let import_at = rendered.find("[import]").unwrap();
        let location_at = rendered.find("[location]").unwrap();
        assert!(tags_at < dates_at);
        assert!(dates_at < import_at);
        assert!(import_at < location_at);
        assert!(rendered.contains("\n+++\n\n# Body\n"));

        let (front_matter, _) = split_front_matter(&rendered);
        let parsed = front_matter_fields(front_matter.unwrap());
        assert_eq!(parsed.location, fm.location);
        assert_eq!(parsed.metadata.tags, vec!["dream".to_string()]);
        assert_eq!(parsed.import, fm.import);
        assert_eq!(
            parsed.datetime.created_at.as_deref(),
            Some("2021-04-03T08:30:05+02:00")
        );
    }

    #[test]
    fn weather_and_celestial_tables_serialize_and_round_trip() {
        // The gate: `[weather]` (with a nested `[weather.wind]`) and `[celestial]`
        // must serialize after `[location]` and re-parse. The nested sub-table
        // under the flattened FrontMatter is the risky bit.
        let fm = FrontMatter {
            location: Some(Location {
                locality: Some("Testville".to_string()),
                ..Location::default()
            }),
            weather: Some(Weather {
                condition: Some("partly-cloudy".to_string()),
                temperature_celsius: Some(19.9),
                wind: Some(Wind {
                    speed_kph: Some(12.0),
                    direction: Some(210.0),
                }),
                ..Weather::default()
            }),
            celestial: Some(Celestial {
                moon_phase: Some(0.5),
                moon_phase_name: Some("full".to_string()),
                ..Celestial::default()
            }),
            ..FrontMatter::default()
        };

        let rendered = render_entry(&fm, "# Body\n");
        // Ordering: [location] then [weather], its sub-table, then [celestial].
        let location_at = rendered.find("[location]").unwrap();
        let weather_at = rendered.find("[weather]").unwrap();
        let wind_at = rendered.find("[weather.wind]").unwrap();
        let celestial_at = rendered.find("[celestial]").unwrap();
        assert!(location_at < weather_at, "{rendered}");
        assert!(weather_at < wind_at);
        assert!(wind_at < celestial_at);

        let (front_matter, _) = split_front_matter(&rendered);
        let parsed = front_matter_fields(front_matter.unwrap());
        assert_eq!(parsed.weather, fm.weather);
        assert_eq!(parsed.celestial, fm.celestial);
    }

    #[test]
    fn timezone_is_preserved_across_metadata_edits() {
        let content = "+++\n[datetime]\ncreated_at = \"2021-04-03T08:30:05+02:00\"\ntimezone = \"Europe/Berlin\"\n+++\n\n# Body\n";

        // A metadata edit re-renders the whole front matter; the capture-only
        // timezone must survive untouched, like the import provenance does.
        let updated =
            with_metadata_field(content, &MetadataField::Tags(vec!["x".to_string()])).unwrap();

        assert_eq!(
            front_matter_fields(split_front_matter(&updated).0.unwrap())
                .datetime
                .timezone,
            Some("Europe/Berlin".to_string())
        );
    }

    #[test]
    fn add_writing_seconds_accumulates_without_touching_edited_at() {
        let content = "+++\n[datetime]\ncreated_at = \"2021-04-03T08:30:05+02:00\"\nedited_at = \"2021-04-03T08:30:05+02:00\"\n+++\n\n# Body\n";

        let once = add_writing_seconds(content, 60).unwrap();
        let after_once = front_matter_fields(split_front_matter(&once).0.unwrap());
        assert_eq!(after_once.datetime.writing_seconds, Some(60));
        // Recording editing time must not look like a content edit.
        assert_eq!(
            after_once.datetime.edited_at.as_deref(),
            Some("2021-04-03T08:30:05+02:00")
        );

        // A second session accumulates on top of the first.
        let twice = add_writing_seconds(&once, 30).unwrap();
        assert_eq!(
            front_matter_fields(split_front_matter(&twice).0.unwrap())
                .datetime
                .writing_seconds,
            Some(90)
        );

        // Omitted from the rendered TOML when never set.
        let no_field = with_metadata_field(content, &MetadataField::Tags(vec![])).unwrap();
        assert!(!no_field.contains("writing_seconds"));
    }

    #[test]
    fn starred_round_trips_and_omits_when_false() {
        let content =
            "+++\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Body\n";

        let starred = with_metadata_field(content, &MetadataField::Starred(true)).unwrap();
        assert!(starred.contains("starred = true"));
        assert!(
            front_matter_fields(split_front_matter(&starred).0.unwrap())
                .metadata
                .starred
        );

        let unstarred = with_metadata_field(&starred, &MetadataField::Starred(false)).unwrap();
        // A false flag leaves no key behind.
        assert!(!unstarred.contains("starred"));
        assert!(
            !front_matter_fields(split_front_matter(&unstarred).0.unwrap())
                .metadata
                .starred
        );
    }

    #[test]
    fn empty_metadata_lists_are_omitted_from_front_matter() {
        let rendered = render_entry(&FrontMatter::default(), "# Body\n");

        for key in ["tags", "people", "activities", "feelings"] {
            assert!(
                !rendered.contains(key),
                "empty `{key}` should not appear in front matter:\n{rendered}"
            );
        }

        // A non-empty list is still written.
        let mut fm = FrontMatter::default();
        fm.metadata.tags = vec!["work".to_string()];
        assert!(render_entry(&fm, "# Body\n").contains("tags = [\"work\"]"));
    }

    #[test]
    fn malformed_front_matter_returns_empty_metadata() {
        assert_eq!(
            front_matter_fields("tags = [unterminated").metadata.tags,
            Vec::<String>::new()
        );
        assert_eq!(
            front_matter_fields("[datetime]\ncreated_at = [unterminated")
                .datetime
                .created_at,
            None
        );
    }

    #[test]
    fn with_metadata_field_replaces_list_without_stale_entries() {
        let content = "+++\ntags = [\"old\", \"stale\"]\n\n[datetime]\ncreated_at = \"2026-07-01T10:00:00+02:00\"\n+++\n\n# Body\n";
        let tags = vec!["new".to_string(), "next".to_string()];

        let updated = with_metadata_field(content, &MetadataField::Tags(tags)).unwrap();

        let (front_matter, _) = split_front_matter(&updated);
        assert_eq!(
            front_matter.map(|fm| front_matter_fields(fm).metadata.tags),
            Some(vec!["new".to_string(), "next".to_string()])
        );
        assert!(!updated.contains("old"));
        assert!(!updated.contains("stale"));
        assert!(updated.contains("\n+++\n\n# Body\n"));
        assert!(updated.ends_with("\n# Body\n"));
    }

    #[test]
    fn with_metadata_field_refreshes_edited_at_and_preserves_body() {
        let content =
            "+++\ntags = []\n\n[datetime]\ncreated_at = \"old\"\n+++\n\n# Body\n\nTrailing\n";

        let updated =
            with_metadata_field(content, &MetadataField::Feelings(vec!["calm".to_string()]))
                .unwrap();

        assert!(updated.contains("\n+++\n\n# Body\n"));
        assert!(updated.ends_with("\n# Body\n\nTrailing\n"));
        assert_eq!(
            front_matter_fields(split_front_matter(&updated).0.unwrap())
                .metadata
                .feelings,
            vec!["calm".to_string()]
        );
        assert!(
            front_matter_fields(split_front_matter(&updated).0.unwrap())
                .datetime
                .edited_at
                .is_some()
        );
    }
}
