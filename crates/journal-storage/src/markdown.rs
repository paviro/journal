use journal_core::{Metadata, MetadataField};
use serde::{Deserialize, Serialize};

/// Every entry front-matter field, parsed and serialized in a single TOML pass.
/// The user metadata is the shared [`Metadata`] type, flattened so its fields
/// sit at the top level of the front matter (mood clamped on read there).
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct FrontMatter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub metadata: Metadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_id: Option<String>,
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
/// matter and `updated_at` refreshed. `None` when there is no front matter.
pub fn with_metadata_field(content: &str, field: &MetadataField) -> Option<String> {
    map_front_matter(content, |fm| {
        match field {
            MetadataField::Tags(values) => fm.metadata.tags = values.clone(),
            MetadataField::People(values) => fm.metadata.people = values.clone(),
            MetadataField::Activities(values) => fm.metadata.activities = values.clone(),
            MetadataField::Feelings(values) => fm.metadata.feelings = values.clone(),
            MetadataField::Mood(mood) => fm.metadata.mood = *mood,
        }
        fm.updated_at = Some(chrono::Local::now().to_rfc3339());
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
        let content = "+++\ncreated_at = \"x\"\n+++\n\n# Body\n";

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
    fn malformed_front_matter_returns_empty_metadata() {
        assert_eq!(
            front_matter_fields("tags = [unterminated").metadata.tags,
            Vec::<String>::new()
        );
        assert_eq!(
            front_matter_fields("created_at = [unterminated").created_at,
            None
        );
    }

    #[test]
    fn with_metadata_field_replaces_list_without_stale_entries() {
        let content = "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\ntags = [\"old\", \"stale\"]\n+++\n\n# Body\n";
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
    fn with_metadata_field_refreshes_updated_at_and_preserves_body() {
        let content = "+++\ncreated_at = \"old\"\ntags = []\n+++\n\n# Body\n\nTrailing\n";

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
                .updated_at
                .is_some()
        );
    }
}
