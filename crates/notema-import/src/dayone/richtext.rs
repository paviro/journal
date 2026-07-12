//! Render Day One's `richText` into Markdown.
//!
//! Newer Day One entries carry a `richText` field in the JSON export: a JSON
//! *string* (Day One's `ZRICHTEXTJSON`) holding a clean, structured body. It is
//! far more faithful than the sibling `text` field, which is Day One's lossy
//! Markdown rendering — code blocks shredded into one fence per line, emphasis
//! wrapped in zero-width spaces, punctuation backslash-escaped. When `richText`
//! is present the importer renders from it; otherwise it falls back to cleaning
//! up `text` (see [`crate::dayone::text`]).
//!
//! Shape: `{"contents": [Run, ...]}`. A run is either a text run (`text`, with
//! optional `attributes`) or an embed run (`embeddedObjects`). Lines are
//! delimited by `\n` *within* the concatenated run text, not by run boundaries,
//! so line-level attributes (header/list/quote/codeBlock) attach to whichever run
//! carries the line, while inline attributes (bold/italic/…) wrap a single run.
//! Photo/audio embeds become `dayone-moment://<id>` references so the existing
//! [`crate::dayone::moments::rewrite_moments`] step resolves them like any other
//! moment.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RichText {
    #[serde(default)]
    contents: Vec<Run>,
}

#[derive(Debug, Deserialize)]
struct Run {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    attributes: Option<Attributes>,
    #[serde(default, rename = "embeddedObjects")]
    embedded_objects: Vec<Embed>,
}

#[derive(Debug, Default, Deserialize)]
struct Attributes {
    #[serde(default)]
    line: Option<LineAttr>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    strikethrough: bool,
    #[serde(default, rename = "inlineCode")]
    inline_code: bool,
    #[serde(default, rename = "linkURL")]
    link_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct LineAttr {
    #[serde(default)]
    header: Option<u8>,
    #[serde(default)]
    quote: bool,
    #[serde(default, rename = "listStyle")]
    list_style: Option<String>,
    #[serde(default, rename = "listIndex")]
    list_index: Option<u64>,
    #[serde(default)]
    checked: bool,
    #[serde(default, rename = "indentLevel")]
    indent_level: Option<u32>,
    #[serde(default, rename = "codeBlock")]
    code_block: bool,
}

#[derive(Debug, Deserialize)]
struct Embed {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    identifier: Option<String>,
    #[serde(default)]
    markdown: Option<String>,
}

/// How a finished line is prefixed. Mirrors the subset of Day One line attributes
/// we render; `Code` lines are grouped into a single fenced block.
#[derive(Debug, PartialEq, Eq)]
enum LineKind {
    Plain,
    Header(u8),
    Quote,
    Bullet(u32),
    Numbered(u32, u64),
    Check(u32, bool),
    Code,
    /// A block-level embed (image / horizontal rule / table). Rendered with a
    /// blank line on each side so it is its own Markdown block.
    Block,
}

/// Render a Day One `richText` JSON string to Markdown. Returns `None` if the
/// string does not parse or yields no content, so the caller can fall back to the
/// `text` cleanup path.
pub(crate) fn render(json: &str) -> Option<String> {
    let doc: RichText = serde_json::from_str(json).ok()?;

    let mut lines: Vec<(LineKind, String)> = Vec::new();
    let mut cur = String::new();
    let mut cur_kind = LineKind::Plain;

    for run in &doc.contents {
        if !run.embedded_objects.is_empty() {
            flush(&mut lines, &mut cur, &mut cur_kind);
            for embed in &run.embedded_objects {
                if let Some(rendered) = render_embed(embed) {
                    lines.push((LineKind::Block, rendered));
                }
            }
            continue;
        }

        let attrs = run.attributes.as_ref();
        if let Some(line) = attrs.and_then(|a| a.line.as_ref()) {
            cur_kind = line_kind(line);
        }

        let Some(text) = run.text.as_deref() else {
            continue;
        };
        let mut segments = text.split('\n').peekable();
        while let Some(segment) = segments.next() {
            cur.push_str(&apply_inline(segment, attrs));
            if segments.peek().is_some() {
                // A `\n` follows this segment: finish the current line.
                lines.push((
                    std::mem::replace(&mut cur_kind, LineKind::Plain),
                    std::mem::take(&mut cur),
                ));
            }
        }
    }
    flush(&mut lines, &mut cur, &mut cur_kind);

    let body = assemble(&lines);
    (!body.trim().is_empty()).then_some(body)
}

/// Push the in-progress line if it carries any content or a non-plain kind, then
/// reset. Avoids emitting a spurious trailing blank line.
fn flush(lines: &mut Vec<(LineKind, String)>, cur: &mut String, cur_kind: &mut LineKind) {
    if !cur.is_empty() || *cur_kind != LineKind::Plain {
        lines.push((
            std::mem::replace(cur_kind, LineKind::Plain),
            std::mem::take(cur),
        ));
    }
}

fn line_kind(line: &LineAttr) -> LineKind {
    let indent = line.indent_level.unwrap_or(1).max(1);
    match line.list_style.as_deref() {
        Some("bulleted") => return LineKind::Bullet(indent),
        Some("numbered") => return LineKind::Numbered(indent, line.list_index.unwrap_or(1)),
        Some("checkbox") => return LineKind::Check(indent, line.checked),
        _ => {}
    }
    if line.code_block {
        LineKind::Code
    } else if line.quote {
        LineKind::Quote
    } else if let Some(level) = line.header.filter(|&h| (1..=6).contains(&h)) {
        LineKind::Header(level)
    } else {
        LineKind::Plain
    }
}

/// Wrap a text segment in its inline Markdown marks. Empty/whitespace segments
/// are left bare so we never emit `** **`.
fn apply_inline(segment: &str, attrs: Option<&Attributes>) -> String {
    let Some(attrs) = attrs else {
        return segment.to_string();
    };
    if segment.trim().is_empty() {
        return segment.to_string();
    }
    let mut out = segment.to_string();
    if attrs.inline_code {
        out = format!("`{out}`");
    }
    if attrs.strikethrough {
        out = format!("~~{out}~~");
    }
    if attrs.italic {
        out = format!("*{out}*");
    }
    if attrs.bold {
        out = format!("**{out}**");
    }
    if let Some(url) = &attrs.link_url {
        out = format!("[{out}]({url})");
    }
    out
}

fn render_embed(embed: &Embed) -> Option<String> {
    match embed.kind.as_deref() {
        Some("horizontalRuleLine") => Some("---".to_string()),
        Some("table") => embed.markdown.clone().filter(|m| !m.is_empty()),
        // photo/audio/video/pdf and any other identified moment.
        _ => embed
            .identifier
            .as_deref()
            .map(|id| format!("![](dayone-moment://{id})")),
    }
}

/// Turn the line list into Markdown. Consecutive code lines collapse into one
/// fenced block; block embeds get a blank line on each side; and runs of blank
/// lines outside code are collapsed to a single blank.
fn assemble(lines: &[(LineKind, String)]) -> String {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        match &lines[i].0 {
            LineKind::Code => {
                // Emit code lines verbatim (blank lines inside a block are
                // significant, so bypass the blank-collapsing pushes).
                out.push("```".to_string());
                while i < lines.len() && lines[i].0 == LineKind::Code {
                    out.push(lines[i].1.clone());
                    i += 1;
                }
                out.push("```".to_string());
            }
            LineKind::Block => {
                push_line(&mut out, String::new());
                push_line(&mut out, lines[i].1.clone());
                push_line(&mut out, String::new());
                i += 1;
            }
            kind => {
                push_line(&mut out, format!("{}{}", prefix(kind), lines[i].1));
                i += 1;
            }
        }
    }
    let mut body = out.join("\n");
    body.truncate(body.trim_end().len());
    body
}

/// Push a line, dropping a blank that would follow another blank (or lead the
/// document) so block padding never stacks into multiple empty lines.
fn push_line(out: &mut Vec<String>, line: String) {
    if line.is_empty() && out.last().map(String::is_empty).unwrap_or(true) {
        return;
    }
    out.push(line);
}

fn prefix(kind: &LineKind) -> String {
    let indent = |level: u32| "\t".repeat(level.saturating_sub(1) as usize);
    match kind {
        LineKind::Plain | LineKind::Code | LineKind::Block => String::new(),
        LineKind::Header(level) => format!("{} ", "#".repeat(*level as usize)),
        LineKind::Quote => "> ".to_string(),
        LineKind::Bullet(level) => format!("{}- ", indent(*level)),
        LineKind::Numbered(level, index) => format!("{}{index}. ", indent(*level)),
        LineKind::Check(level, checked) => {
            format!(
                "{}- [{}] ",
                indent(*level),
                if *checked { "x" } else { " " }
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_header_paragraph_and_inline_marks() {
        let json = r#"{"contents":[
            {"attributes":{"line":{"header":1}},"text":"Title\n"},
            {"text":"\nplain and "},
            {"attributes":{"bold":true},"text":"bold"},
            {"text":" and "},
            {"attributes":{"italic":true},"text":"italic"},
            {"text":" and a "},
            {"attributes":{"linkURL":"https://x.test"},"text":"link"},
            {"text":".\n"}
        ]}"#;
        assert_eq!(
            render(json).unwrap(),
            "# Title\n\nplain and **bold** and *italic* and a [link](https://x.test)."
        );
    }

    #[test]
    fn groups_consecutive_code_lines_into_one_fence() {
        let json = r#"{"contents":[
            {"attributes":{"line":{"codeBlock":true}},"text":"line one\n"},
            {"attributes":{"line":{"codeBlock":true}},"text":"line two\n"},
            {"attributes":{"line":{"codeBlock":true}},"text":"\n"},
            {"attributes":{"line":{"codeBlock":true}},"text":"line three\n"},
            {"text":"after\n"}
        ]}"#;
        assert_eq!(
            render(json).unwrap(),
            "```\nline one\nline two\n\nline three\n```\nafter"
        );
    }

    #[test]
    fn renders_lists_with_nesting_and_checkboxes() {
        let json = r#"{"contents":[
            {"attributes":{"line":{"listStyle":"numbered","listIndex":1}},"text":"first\n"},
            {"attributes":{"line":{"listStyle":"numbered","listIndex":2,"indentLevel":2}},"text":"nested\n"},
            {"attributes":{"line":{"listStyle":"bulleted"}},"text":"bullet\n"},
            {"attributes":{"line":{"listStyle":"checkbox","checked":true}},"text":"done\n"},
            {"attributes":{"line":{"listStyle":"checkbox"}},"text":"todo\n"}
        ]}"#;
        assert_eq!(
            render(json).unwrap(),
            "1. first\n\t2. nested\n- bullet\n- [x] done\n- [ ] todo"
        );
    }

    #[test]
    fn renders_quote_lines() {
        let json = r#"{"contents":[
            {"attributes":{"line":{"quote":true}},"text":"quoted\n"},
            {"text":"plain\n"}
        ]}"#;
        assert_eq!(render(json).unwrap(), "> quoted\nplain");
    }

    #[test]
    fn renders_embeds_photo_rule_and_table() {
        let json = r#"{"contents":[
            {"embeddedObjects":[{"identifier":"PHOTO1","type":"photo"}]},
            {"text":"\n"},
            {"embeddedObjects":[{"type":"horizontalRuleLine"}]},
            {"text":"\n"},
            {"embeddedObjects":[{"type":"table","markdown":"| a | b |\n| --- | --- |"}]}
        ]}"#;
        assert_eq!(
            render(json).unwrap(),
            "![](dayone-moment://PHOTO1)\n\n---\n\n| a | b |\n| --- | --- |"
        );
    }

    #[test]
    fn block_embeds_get_blank_line_separation() {
        // Day One stores adjacent photos and a caption as block runs with no
        // explicit blank lines; they must still render as separate blocks.
        let json = r#"{"contents":[
            {"embeddedObjects":[{"identifier":"P1","type":"photo"}]},
            {"text":"caption"},
            {"embeddedObjects":[{"identifier":"P2","type":"photo"}]}
        ]}"#;
        assert_eq!(
            render(json).unwrap(),
            "![](dayone-moment://P1)\n\ncaption\n\n![](dayone-moment://P2)"
        );
    }

    #[test]
    fn invalid_or_empty_json_returns_none() {
        assert_eq!(render("not json"), None);
        assert_eq!(render(r#"{"contents":[]}"#), None);
        assert_eq!(render(r#"{"contents":[{"text":"   \n"}]}"#), None);
    }
}
