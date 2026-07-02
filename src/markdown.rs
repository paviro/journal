use noyalib::{Mapping, Value};

pub fn split_front_matter(content: &str) -> (Option<&str>, &str) {
    let Some(rest) = content.strip_prefix("---\n") else {
        return (None, content);
    };

    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let marker = line.trim_end_matches('\n').trim_end_matches('\r');
        if marker == "..." {
            let front_matter = rest[..offset].trim_end_matches('\n').trim_end_matches('\r');
            let body = &rest[offset + line.len()..];
            return (Some(front_matter), body);
        }
        offset += line.len();
    }

    if let Some(index) = rest.rfind('\n') {
        let marker = &rest[index + 1..];
        if marker == "..." {
            let front_matter = rest[..index].trim_end_matches('\r');
            return (Some(front_matter), "");
        }
    }

    (None, content)
}

pub fn front_matter_tags(front_matter: &str) -> Vec<String> {
    parse_front_matter(front_matter)
        .and_then(|metadata| metadata.get("tags").map(strings_from_value))
        .unwrap_or_default()
}

pub fn front_matter_feelings(front_matter: &str) -> Vec<String> {
    parse_front_matter(front_matter)
        .and_then(|metadata| metadata.get("feelings").map(strings_from_value))
        .unwrap_or_default()
}

pub fn front_matter_mood(front_matter: &str) -> Option<i8> {
    parse_front_matter(front_matter)
        .and_then(|metadata| metadata.get("mood").and_then(|v| v.as_i64()))
        .filter(|&v| (-5..=5).contains(&v))
        .map(|v| v as i8)
}

pub fn front_matter_value(front_matter: &str, key: &str) -> Option<String> {
    parse_front_matter(front_matter).and_then(|metadata| {
        metadata
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

pub fn entry_has_body(content: &str) -> bool {
    let (_, body) = split_front_matter(content);
    !body.trim().is_empty()
}

pub fn display_title_and_preview(body: &str, timestamp_fallback: &str) -> (String, String) {
    let mut lines = body.lines().filter_map(display_line_text);
    let title = lines
        .next()
        .filter(|line| !line.is_empty())
        .unwrap_or(timestamp_fallback)
        .to_string();
    let preview = lines.next().map(truncate_preview).unwrap_or_default();

    (title, preview)
}

pub(crate) fn set_front_matter_value(content: &str, key: &str, value: &str) -> String {
    let (front_matter, body) = split_front_matter(content);
    let Some(front_matter) = front_matter else {
        return content.to_string();
    };

    let Some(mut metadata) = parse_front_matter(front_matter) else {
        return content.to_string();
    };
    metadata.insert(key, Value::from(value));
    render_content_with_front_matter(&metadata, body)
}

/// Replace the `tags` field in the YAML front matter with the given list.
/// Returns `None` when there is no front matter.
pub(crate) fn set_tags_in_front_matter(content: &str, tags: &[String]) -> Option<String> {
    set_string_list_in_front_matter(content, "tags", tags)
}

/// Replace the `feelings` field in the YAML front matter with the given list.
/// Returns `None` when there is no front matter.
pub(crate) fn set_feelings_in_front_matter(content: &str, feelings: &[String]) -> Option<String> {
    set_string_list_in_front_matter(content, "feelings", feelings)
}

/// Set or remove the `mood` field in the YAML front matter.
/// Returns `None` when there is no front matter.
pub(crate) fn set_mood_in_front_matter(content: &str, mood: Option<i8>) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let front_matter = front_matter?;
    let mut metadata = parse_front_matter(front_matter)?;
    match mood {
        Some(value) => {
            metadata.insert("mood", Value::from(value));
        }
        None => {
            metadata.remove("mood");
        }
    }
    Some(render_content_with_front_matter(&metadata, body))
}

fn set_string_list_in_front_matter(content: &str, key: &str, values: &[String]) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let front_matter = front_matter?;

    let mut metadata = parse_front_matter(front_matter)?;
    metadata.insert(
        key,
        Value::Sequence(
            values
                .iter()
                .map(|value| Value::from(value.as_str()))
                .collect(),
        ),
    );

    Some(render_content_with_front_matter(&metadata, body))
}

fn parse_front_matter(front_matter: &str) -> Option<Mapping> {
    let value: Value = noyalib::from_str(front_matter).ok()?;
    match value {
        Value::Mapping(metadata) => Some(metadata),
        _ => None,
    }
}

fn strings_from_value(value: &Value) -> Vec<String> {
    if let Some(value) = value.as_str() {
        return non_empty_string(value).into_iter().collect();
    }

    value
        .as_sequence()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().and_then(non_empty_string))
                .collect()
        })
        .unwrap_or_default()
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn render_content_with_front_matter(metadata: &Mapping, body: &str) -> String {
    let mut rendered = noyalib::to_string(&Value::Mapping(metadata.clone())).unwrap_or_default();
    if let Some(front_matter) = rendered.strip_prefix("---\n") {
        rendered = front_matter.to_string();
    }
    if let Some(front_matter) = rendered.strip_suffix("...\n") {
        rendered = front_matter.to_string();
    }
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    format!("---\n{}...\n{}", rendered, body)
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
    line.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_title_uses_heading_then_next_line_as_preview() {
        let (title, preview) = display_title_and_preview("## Real Title\nBody text", "timestamp");

        assert_eq!(title, "Real Title");
        assert_eq!(preview, "Body text");
    }

    #[test]
    fn display_title_falls_back_to_timestamp_when_empty() {
        let (title, preview) = display_title_and_preview("\n\n", "2026-07-01T23:30:00+02:00");

        assert_eq!(title, "2026-07-01T23:30:00+02:00");
        assert_eq!(preview, "");
    }

    #[test]
    fn split_front_matter_accepts_yaml_document_end_marker() {
        let (front_matter, body) = split_front_matter("---\ntitle: \"A\"\n...\n\n# Body\n");

        assert_eq!(front_matter, Some("title: \"A\""));
        assert_eq!(body, "\n# Body\n");
    }

    #[test]
    fn front_matter_tags_reads_block_list() {
        let tags = front_matter_tags("tags:\n  - foo\n  - bar\n");

        assert_eq!(tags, vec!["foo", "bar"]);
    }

    #[test]
    fn front_matter_tags_reads_flow_list_with_quoted_commas() {
        let tags = front_matter_tags("tags: [\"foo, bar\", baz]\n");

        assert_eq!(tags, vec!["foo, bar", "baz"]);
    }

    #[test]
    fn front_matter_tags_keeps_scalar_tag_compatibility() {
        let tags = front_matter_tags("tags: foo\n");

        assert_eq!(tags, vec!["foo"]);
    }

    #[test]
    fn front_matter_feelings_reads_block_list() {
        let feelings = front_matter_feelings("feelings:\n  - calm\n  - focused\n");

        assert_eq!(feelings, vec!["calm", "focused"]);
    }

    #[test]
    fn front_matter_feelings_keeps_scalar_compatibility() {
        let feelings = front_matter_feelings("feelings: calm\n");

        assert_eq!(feelings, vec!["calm"]);
    }

    #[test]
    fn malformed_front_matter_returns_empty_metadata() {
        assert_eq!(
            front_matter_tags("tags: [unterminated"),
            Vec::<String>::new()
        );
        assert_eq!(
            front_matter_value("created_at: [unterminated", "created_at"),
            None
        );
    }

    #[test]
    fn set_tags_replaces_block_list_without_stale_rows() {
        let content = "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\ntags:\n  - old\n  - stale\n...\n\n# Body\n";
        let tags = vec!["new".to_string(), "next".to_string()];

        let updated = set_tags_in_front_matter(content, &tags).unwrap();

        let (front_matter, _) = split_front_matter(&updated);
        assert_eq!(
            front_matter.map(front_matter_tags),
            Some(vec!["new".to_string(), "next".to_string()])
        );
        assert!(!updated.contains("old"));
        assert!(!updated.contains("stale"));
        assert!(updated.contains("\n...\n\n# Body\n"));
        assert!(updated.ends_with("\n# Body\n"));
    }

    #[test]
    fn set_feelings_replaces_block_list_without_stale_rows() {
        let content = "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\nfeelings:\n  - tired\n  - stale\n...\n\n# Body\n";
        let feelings = vec!["calm".to_string(), "focused".to_string()];

        let updated = set_feelings_in_front_matter(content, &feelings).unwrap();

        let (front_matter, _) = split_front_matter(&updated);
        assert_eq!(
            front_matter.map(front_matter_feelings),
            Some(vec!["calm".to_string(), "focused".to_string()])
        );
        assert!(!updated.contains("stale"));
        assert!(updated.contains("\n...\n\n# Body\n"));
    }

    #[test]
    fn set_front_matter_value_preserves_body_exactly() {
        let content = "---\ncreated_at: \"old\"\ntags: []\n...\n\n# Body\n\nTrailing\n";

        let updated = set_front_matter_value(content, "updated_at", "new");

        assert!(updated.contains("\n...\n\n# Body\n"));
        assert!(updated.ends_with("\n# Body\n\nTrailing\n"));
        assert_eq!(
            front_matter_value(split_front_matter(&updated).0.unwrap(), "updated_at"),
            Some("new".to_string())
        );
    }
}
