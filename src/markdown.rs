pub fn split_front_matter(content: &str) -> (Option<&str>, &str) {
    let Some(rest) = content.strip_prefix("---\n") else {
        return (None, content);
    };

    if let Some(index) = rest.find("\n---\n") {
        let front_matter = &rest[..index];
        let body = &rest[index + "\n---\n".len()..];
        return (Some(front_matter), body);
    }

    (None, content)
}

pub fn front_matter_tags(front_matter: &str) -> Vec<String> {
    front_matter
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            let list = trimmed.strip_prefix("tags:")?.trim();
            if list.is_empty() || list == "[]" {
                return Some(Vec::new());
            }
            let inner = list
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or(list);
            let tags: Vec<String> = inner
                .split(',')
                .filter_map(|t| {
                    let t = t.trim().trim_matches('"').trim().to_string();
                    if t.is_empty() { None } else { Some(t) }
                })
                .collect();
            Some(tags)
        })
        .unwrap_or_default()
}

pub fn front_matter_value(front_matter: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    front_matter.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&prefix)?.trim();
        Some(value.trim_matches('"').to_string())
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

    let prefix = format!("{key}:");
    let mut found = false;
    let mut lines = Vec::new();
    for line in front_matter.lines() {
        if line.trim_start().starts_with(&prefix) {
            lines.push(format!("{key}: \"{}\"", escape_yaml_string(value)));
            found = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !found {
        lines.push(format!("{key}: \"{}\"", escape_yaml_string(value)));
    }

    format!("---\n{}\n---\n{}", lines.join("\n"), body)
}

/// Serialize a list of tags into a `tags: [...]` line.
fn serialize_tags(tags: &[String]) -> String {
    format!(
        "[{}]",
        tags.iter()
            .map(|t| format!("\"{}\"", escape_yaml_string(t)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Replace the `tags` field in the YAML front matter with the given list.
/// Returns `None` when there is no front matter.
pub(crate) fn set_tags_in_front_matter(content: &str, tags: &[String]) -> Option<String> {
    let (front_matter, body) = split_front_matter(content);
    let front_matter = front_matter?;

    let serialized = serialize_tags(tags);

    let prefix = "tags:";
    let mut found = false;
    let mut lines: Vec<String> = front_matter
        .lines()
        .map(|line| {
            if line.trim_start().starts_with(prefix) {
                found = true;
                format!("tags: {serialized}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(format!("tags: {serialized}"));
    }

    Some(format!("---\n{}\n---\n{}", lines.join("\n"), body))
}

pub(crate) fn escape_yaml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
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
    fn split_front_matter_returns_body_after_yaml() {
        let (front_matter, body) = split_front_matter("---\ntitle: \"A\"\n---\n\n# Body\n");

        assert_eq!(front_matter, Some("title: \"A\""));
        assert_eq!(body, "\n# Body\n");
    }
}
