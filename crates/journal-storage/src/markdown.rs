use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone)]
struct FrontMatter {
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    people: Vec<String>,
    #[serde(default)]
    activities: Vec<String>,
    #[serde(default)]
    feelings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mood: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    import_id: Option<String>,
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

/// Clamp a raw TOML `mood` integer to the supported `-5..=5` range, dropping
/// out-of-range or non-`i8` values.
fn clamp_mood(mood: Option<i64>) -> Option<i8> {
    mood.and_then(|v| i8::try_from(v).ok())
        .filter(|&v| (-5..=5).contains(&v))
}

/// Every front-matter field, parsed in a single `toml::from_str` pass. The load
/// path builds one of these per entry instead of calling the eight individual
/// `front_matter_*` accessors, each of which re-parses the whole TOML block —
/// so a full corpus load parses each entry's front matter once, not ~8 times.
#[derive(Default)]
pub struct FrontMatterFields {
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub tags: Vec<String>,
    pub people: Vec<String>,
    pub activities: Vec<String>,
    pub feelings: Vec<String>,
    pub mood: Option<i8>,
    pub import_id: Option<String>,
}

/// Parse every front-matter field at once. Malformed TOML yields defaults,
/// matching the lenient behavior of the individual accessors.
pub fn front_matter_fields(front_matter: &str) -> FrontMatterFields {
    let Some(fm) = parse_front_matter(front_matter) else {
        return FrontMatterFields::default();
    };
    FrontMatterFields {
        created_at: fm.created_at,
        updated_at: fm.updated_at,
        tags: fm.tags,
        people: fm.people,
        activities: fm.activities,
        feelings: fm.feelings,
        mood: clamp_mood(fm.mood),
        import_id: fm.import_id,
    }
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
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;

    while i < chars.len() {
        let is_image = chars[i] == '!' && chars.get(i + 1) == Some(&'[');
        let bracket = if is_image { i + 1 } else { i };
        if (is_image || chars[i] == '[')
            && let Some(close) = find_char(&chars, bracket + 1, ']')
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = find_char(&chars, close + 2, ')')
        {
            out.push_str(if is_image { "[image]" } else { "[link]" });
            i = end + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

fn find_char(chars: &[char], start: usize, target: char) -> Option<usize> {
    chars[start..]
        .iter()
        .position(|&c| c == target)
        .map(|offset| start + offset)
}

pub fn set_front_matter_value(content: &str, key: &str, value: &str) -> String {
    let (front_matter, body) = split_front_matter(content);
    let Some(front_matter) = front_matter else {
        return content.to_string();
    };
    let Some(mut metadata) = parse_front_matter(front_matter) else {
        return content.to_string();
    };
    match key {
        "created_at" => metadata.created_at = Some(value.to_string()),
        "updated_at" => metadata.updated_at = Some(value.to_string()),
        "import_id" => metadata.import_id = Some(value.to_string()),
        _ => return content.to_string(),
    }
    render_content_with_front_matter(&metadata, body)
}

/// Replace the `tags` field in the TOML front matter with the given list.
/// Returns `None` when there is no front matter.
pub fn set_tags_in_front_matter(content: &str, tags: &[String]) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let front_matter = front_matter?;
    let mut metadata = parse_front_matter(front_matter)?;
    metadata.tags = tags.to_vec();
    Some(render_content_with_front_matter(&metadata, body))
}

/// Replace the `people` field in the TOML front matter with the given list.
/// Returns `None` when there is no front matter.
pub fn set_people_in_front_matter(content: &str, people: &[String]) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let front_matter = front_matter?;
    let mut metadata = parse_front_matter(front_matter)?;
    metadata.people = people.to_vec();
    Some(render_content_with_front_matter(&metadata, body))
}

/// Replace the `activities` field in the TOML front matter with the given list.
/// Returns `None` when there is no front matter.
pub fn set_activities_in_front_matter(content: &str, activities: &[String]) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let front_matter = front_matter?;
    let mut metadata = parse_front_matter(front_matter)?;
    metadata.activities = activities.to_vec();
    Some(render_content_with_front_matter(&metadata, body))
}

/// Replace the `feelings` field in the TOML front matter with the given list.
/// Returns `None` when there is no front matter.
pub fn set_feelings_in_front_matter(content: &str, feelings: &[String]) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let front_matter = front_matter?;
    let mut metadata = parse_front_matter(front_matter)?;
    metadata.feelings = feelings.to_vec();
    Some(render_content_with_front_matter(&metadata, body))
}

/// Set or remove the `mood` field in the TOML front matter.
/// Returns `None` when there is no front matter.
pub fn set_mood_in_front_matter(content: &str, mood: Option<i8>) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let front_matter = front_matter?;
    let mut metadata = parse_front_matter(front_matter)?;
    metadata.mood = mood.map(i64::from);
    Some(render_content_with_front_matter(&metadata, body))
}

fn parse_front_matter(front_matter: &str) -> Option<FrontMatter> {
    toml::from_str(front_matter).ok()
}

pub fn set_updated_at_now_in_content(content: &str) -> String {
    set_front_matter_value(content, "updated_at", &chrono::Local::now().to_rfc3339())
}

fn render_content_with_front_matter(metadata: &FrontMatter, body: &str) -> String {
    let toml = toml::to_string(metadata).unwrap_or_default();
    format!("+++\n{}+++\n{}", toml, body)
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
        let tags = front_matter_fields("tags = [\"foo\", \"bar\"]\n").tags;

        assert_eq!(tags, vec!["foo", "bar"]);
    }

    #[test]
    fn front_matter_tags_handles_commas_in_values() {
        let tags = front_matter_fields("tags = [\"foo, bar\", \"baz\"]\n").tags;

        assert_eq!(tags, vec!["foo, bar", "baz"]);
    }

    #[test]
    fn front_matter_feelings_reads_list() {
        let feelings = front_matter_fields("feelings = [\"calm\", \"focused\"]\n").feelings;

        assert_eq!(feelings, vec!["calm", "focused"]);
    }

    #[test]
    fn malformed_front_matter_returns_empty_metadata() {
        assert_eq!(
            front_matter_fields("tags = [unterminated").tags,
            Vec::<String>::new()
        );
        assert_eq!(
            front_matter_fields("created_at = [unterminated").created_at,
            None
        );
    }

    #[test]
    fn set_tags_replaces_list_without_stale_entries() {
        let content = "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\ntags = [\"old\", \"stale\"]\n+++\n\n# Body\n";
        let tags = vec!["new".to_string(), "next".to_string()];

        let updated = set_tags_in_front_matter(content, &tags).unwrap();

        let (front_matter, _) = split_front_matter(&updated);
        assert_eq!(
            front_matter.map(|fm| front_matter_fields(fm).tags),
            Some(vec!["new".to_string(), "next".to_string()])
        );
        assert!(!updated.contains("old"));
        assert!(!updated.contains("stale"));
        assert!(updated.contains("\n+++\n\n# Body\n"));
        assert!(updated.ends_with("\n# Body\n"));
    }

    #[test]
    fn set_feelings_replaces_list_without_stale_entries() {
        let content = "+++\ncreated_at = \"2026-07-01T10:00:00+02:00\"\nfeelings = [\"tired\", \"stale\"]\n+++\n\n# Body\n";
        let feelings = vec!["calm".to_string(), "focused".to_string()];

        let updated = set_feelings_in_front_matter(content, &feelings).unwrap();

        let (front_matter, _) = split_front_matter(&updated);
        assert_eq!(
            front_matter.map(|fm| front_matter_fields(fm).feelings),
            Some(vec!["calm".to_string(), "focused".to_string()])
        );
        assert!(!updated.contains("stale"));
        assert!(updated.contains("\n+++\n\n# Body\n"));
    }

    #[test]
    fn set_front_matter_value_preserves_body_exactly() {
        let content = "+++\ncreated_at = \"old\"\ntags = []\n+++\n\n# Body\n\nTrailing\n";

        let updated = set_front_matter_value(content, "updated_at", "new");

        assert!(updated.contains("\n+++\n\n# Body\n"));
        assert!(updated.ends_with("\n# Body\n\nTrailing\n"));
        assert_eq!(
            front_matter_fields(split_front_matter(&updated).0.unwrap()).updated_at,
            Some("new".to_string())
        );
    }
}
