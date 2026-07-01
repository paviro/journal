use std::borrow::Cow;

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

pub fn front_matter_value(front_matter: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    front_matter.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&prefix)?.trim();
        Some(value.trim_matches('"').to_string())
    })
}

pub fn first_markdown_heading(body: &str) -> Option<&str> {
    body.lines().find_map(|line| {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') {
            return None;
        }

        let after_hashes = trimmed.trim_start_matches('#');
        if after_hashes.starts_with(' ') {
            let title = after_hashes.trim();
            if !title.is_empty() {
                return Some(title);
            }
        }

        None
    })
}

pub fn body_preview(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("---"))
        .map(|line| line.trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
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

pub fn display_title<'a>(body: &'a str, timestamp_fallback: &'a str) -> Cow<'a, str> {
    if let Some(heading) = first_markdown_heading(body) {
        return Cow::Borrowed(heading);
    }

    let preview = body_preview(body);
    if preview.is_empty() {
        Cow::Borrowed(timestamp_fallback)
    } else {
        Cow::Owned(preview)
    }
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
    fn heading_becomes_display_title() {
        assert_eq!(
            first_markdown_heading("Intro\n\n## Real Title\nBody"),
            Some("Real Title")
        );
    }

    #[test]
    fn body_preview_is_title_fallback_when_no_heading_exists() {
        let title = display_title("A plain first sentence.\nMore text.", "timestamp");

        assert_eq!(title, "A plain first sentence.");
    }

    #[test]
    fn empty_body_falls_back_to_creation_timestamp() {
        let title = display_title("\n\n", "2026-07-01T23:30:00+02:00");

        assert_eq!(title, "2026-07-01T23:30:00+02:00");
    }

    #[test]
    fn split_front_matter_returns_body_after_yaml() {
        let (front_matter, body) = split_front_matter("---\ntitle: \"A\"\n---\n\n# Body\n");

        assert_eq!(front_matter, Some("title: \"A\""));
        assert_eq!(body, "\n# Body\n");
    }
}
