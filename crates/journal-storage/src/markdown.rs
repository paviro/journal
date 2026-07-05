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

pub fn front_matter_tags(front_matter: &str) -> Vec<String> {
    parse_front_matter(front_matter)
        .map(|fm| fm.tags)
        .unwrap_or_default()
}

pub fn front_matter_feelings(front_matter: &str) -> Vec<String> {
    parse_front_matter(front_matter)
        .map(|fm| fm.feelings)
        .unwrap_or_default()
}

pub fn front_matter_people(front_matter: &str) -> Vec<String> {
    parse_front_matter(front_matter)
        .map(|fm| fm.people)
        .unwrap_or_default()
}

pub fn front_matter_activities(front_matter: &str) -> Vec<String> {
    parse_front_matter(front_matter)
        .map(|fm| fm.activities)
        .unwrap_or_default()
}

pub fn front_matter_mood(front_matter: &str) -> Option<i8> {
    parse_front_matter(front_matter)
        .and_then(|fm| fm.mood)
        .and_then(|v| i8::try_from(v).ok())
        .filter(|&v| (-5..=5).contains(&v))
}

pub fn front_matter_value(front_matter: &str, key: &str) -> Option<String> {
    let fm = parse_front_matter(front_matter)?;
    match key {
        "created_at" => fm.created_at,
        "updated_at" => fm.updated_at,
        _ => None,
    }
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
    fn split_front_matter_parses_toml_delimiters() {
        let (front_matter, body) = split_front_matter("+++\ntitle = \"A\"\n+++\n\n# Body\n");

        assert_eq!(front_matter, Some("title = \"A\""));
        assert_eq!(body, "\n# Body\n");
    }

    #[test]
    fn front_matter_tags_reads_list() {
        let tags = front_matter_tags("tags = [\"foo\", \"bar\"]\n");

        assert_eq!(tags, vec!["foo", "bar"]);
    }

    #[test]
    fn front_matter_tags_handles_commas_in_values() {
        let tags = front_matter_tags("tags = [\"foo, bar\", \"baz\"]\n");

        assert_eq!(tags, vec!["foo, bar", "baz"]);
    }

    #[test]
    fn front_matter_feelings_reads_list() {
        let feelings = front_matter_feelings("feelings = [\"calm\", \"focused\"]\n");

        assert_eq!(feelings, vec!["calm", "focused"]);
    }

    #[test]
    fn malformed_front_matter_returns_empty_metadata() {
        assert_eq!(
            front_matter_tags("tags = [unterminated"),
            Vec::<String>::new()
        );
        assert_eq!(
            front_matter_value("created_at = [unterminated", "created_at"),
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
            front_matter.map(front_matter_tags),
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
            front_matter.map(front_matter_feelings),
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
            front_matter_value(split_front_matter(&updated).0.unwrap(), "updated_at"),
            Some("new".to_string())
        );
    }
}
