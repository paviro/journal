//! Clean up Day One's lossy `text` (`ZMARKDOWNTEXT`) field into faithful
//! Markdown.
//!
//! This is the fallback body producer, used when an entry has no structured
//! `richText` (see [`crate::dayone::richtext`]). Day One's `text` is heavily
//! mangled — punctuation backslash-escaped, emphasis wrapped in zero-width
//! spaces, code blocks shredded into one fence per line, and older entries
//! carrying raw HTML embeds. The functions here undo each of those, leaving a
//! body whose image references are `dayone-moment://` links for the shared
//! resolver in [`crate::dayone::moments`] to rewrite.

use std::sync::LazyLock;

use regex::Regex;

/// Undo Day One's Markdown escaping: it prefixes literal punctuation with a
/// backslash (e.g. `verdiene\.\.\.`, `\!`, `gaps\.pdf`). We drop the backslash
/// before any ASCII punctuation so the body reads as normal Markdown.
///
/// Heuristic: this also unescapes punctuation inside fenced code blocks, which
/// Day One escapes too. Acceptable given Day One over-escapes.
pub fn unescape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
            && next.is_ascii_punctuation()
        {
            out.push(next);
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// Drop the noise characters Day One injects into its exported Markdown:
/// zero-width spaces (`U+200B`) wrapped around every emphasis run
/// (`​*word*​`), and `U+2028` line separators wedged into list items. The
/// former are pure clutter; the latter is a soft line break, so it becomes a
/// newline.
pub fn normalize_whitespace(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\u{200B}' => None,
            '\u{2028}' => Some('\n'),
            other => Some(other),
        })
        .collect()
}

/// Reassemble Day One's shredded code blocks.
///
/// Day One's exported Markdown (its `ZMARKDOWNTEXT`) renders every code block as
/// *one fenced block per source line* — a five-line snippet becomes five
/// separate ```` ``` ```` blocks — and emits blank lines *inside* a block as
/// empty ```` ``` ``` ```` pairs. It never produces a genuinely multi-line
/// fence. So any maximal run of adjacent fenced blocks (separated only by blank
/// lines) was originally a single code block; concatenating their contents
/// reconstructs it faithfully.
///
/// The one lossy case is two genuinely separate code blocks sitting back-to-back
/// with only blank lines between them: Day One gives us no signal to tell that
/// apart from one block with internal blanks, so they merge into one. Harmless
/// in practice (they still render as code) and rare outside Markdown reference
/// docs.
pub fn merge_code_fences(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if !is_code_fence(lines[i]) {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }
        // Start of a run of shredded fences; collect the reconstructed contents.
        let mut content: Vec<String> = Vec::new();
        loop {
            i += 1; // past the opening fence
            let block_start = content.len();
            while i < lines.len() && !is_code_fence(lines[i]) {
                content.push(lines[i].to_string());
                i += 1;
            }
            if i < lines.len() {
                i += 1; // past the closing fence
            }
            // An empty block encodes a single blank line within the code block.
            if content.len() == block_start {
                content.push(String::new());
            }
            match next_fence_after_blanks(&lines, i) {
                Some(j) => i = j, // drop the blank separators, continue the run
                None => break,
            }
        }
        out.push("```".to_string());
        out.extend(content);
        out.push("```".to_string());
    }
    out.join("\n")
}

/// A whole line that is a code fence: ```` ``` ```` optionally followed by an
/// info string (Day One never adds one, but tolerate it).
fn is_code_fence(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("```")
        && trimmed[3..]
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '+' | '-' | '_'))
}

/// Skipping any blank lines from `start`, return the index of the next line if
/// it is a code fence — meaning the current run continues after cosmetic blank
/// separators. `None` if a non-blank non-fence line (or EOF) ends the run.
fn next_fence_after_blanks(lines: &[&str], start: usize) -> Option<usize> {
    let mut j = start;
    while j < lines.len() && lines[j].trim().is_empty() {
        j += 1;
    }
    (j < lines.len() && is_code_fence(lines[j])).then_some(j)
}

/// Recover the legacy inline HTML embeds older Day One entries carry into clean
/// Markdown. These `<img>`/`<a>`/`<audio>`/`<video>` tags were once rendered by
/// Day One and are still stored verbatim in the export (the current app shows
/// them as code blocks, but that is UI-only — the source is bare HTML, so we
/// treat it as truth and convert):
///
/// - `<a href="H"><img src="S"></a>` → `[![](S)](H)` (linked image)
/// - `<a href="H">text</a>` → `[text](H)`
/// - `<img src="S">` → `![](S)` (dropped if it has no `src`)
/// - `<audio>…<source src="S">…</audio>` → `[audio](S)` (same for `<video>`)
///
/// Fenced code blocks (``` / ~~~) pass through untouched, so tags shown as
/// example markup — e.g. the HTML samples in a Markdown reference entry — stay
/// literal.
pub fn recover_html_embeds(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut region = String::new();
    let mut in_fence = false;
    let mut lines = text.split('\n').peekable();
    while let Some(line) = lines.next() {
        let newline = lines.peek().is_some();
        if is_fence(line) {
            // Flush the pending non-fenced region, then pass the fence verbatim.
            out.push_str(&convert_html_region(&region));
            region.clear();
            in_fence = !in_fence;
            out.push_str(line);
            if newline {
                out.push('\n');
            }
        } else if in_fence {
            out.push_str(line);
            if newline {
                out.push('\n');
            }
        } else {
            // Buffer the whole non-fenced region: audio/video embeds span lines,
            // so conversion cannot be line-by-line.
            region.push_str(line);
            if newline {
                region.push('\n');
            }
        }
    }
    out.push_str(&convert_html_region(&region));
    out
}

fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// First `src="…"`/`src='…'` value inside an HTML tag or block.
static SRC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\bsrc\s*=\s*["']([^"']+)["']"#).unwrap());
static AUDIO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<audio\b.*?</audio>").unwrap());
static VIDEO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<video\b.*?</video>").unwrap());
static IMG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<img\b[^>]*>").unwrap());
static ANCHOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<a\b[^>]*?\bhref\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap()
});

/// Apply the embed conversions to one non-fenced region. Order matters: media
/// blocks first (they contain their own `<source>`), then `<img>`, then `<a>` —
/// so an anchor wrapping an image sees the already-converted `![](…)` and yields
/// a linked image.
fn convert_html_region(region: &str) -> String {
    if region.is_empty() {
        return String::new();
    }
    let media = |kind: &str, block: &str| {
        SRC_RE
            .captures(block)
            .map(|c| format!("[{kind}]({})", &c[1]))
            .unwrap_or_default()
    };
    let step = AUDIO_RE.replace_all(region, |c: &regex::Captures| media("audio", &c[0]));
    let step = VIDEO_RE
        .replace_all(&step, |c: &regex::Captures| media("video", &c[0]))
        .into_owned();
    let step = IMG_RE.replace_all(&step, |c: &regex::Captures| {
        SRC_RE
            .captures(&c[0])
            .map(|src| format!("![]({})", &src[1]))
            .unwrap_or_default()
    });
    ANCHOR_RE
        .replace_all(&step, |c: &regex::Captures| {
            format!("[{}]({})", c[2].trim(), &c[1])
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_strips_backslash_before_punctuation() {
        assert_eq!(unescape_markdown(r"verdiene\.\.\."), "verdiene...");
        assert_eq!(
            unescape_markdown(r"Nice zwei Bilder\!"),
            "Nice zwei Bilder!"
        );
        assert_eq!(unescape_markdown(r"gaps\.pdf"), "gaps.pdf");
        // Backslash before a non-punctuation char is preserved.
        assert_eq!(unescape_markdown(r"a\b"), r"a\b");
    }

    #[test]
    fn normalize_whitespace_strips_zero_width_and_line_separators() {
        // Zero-width spaces Day One wraps emphasis in are dropped.
        assert_eq!(
            normalize_whitespace("with \u{200B}**bold**\u{200B}."),
            "with **bold**."
        );
        // U+2028 line separators become newlines.
        assert_eq!(
            normalize_whitespace("item one\u{2028}item two"),
            "item one\nitem two"
        );
    }

    #[test]
    fn merge_code_fences_reassembles_shredded_block() {
        // Day One renders a multi-line block as one fence per line, with an
        // in-block blank as an empty ``` ``` pair, and cosmetic blank lines
        // (varying counts) between the fences.
        let shredded = "```\n# Install\n```\n\n```\nnpm install\n```\n\n\n```\n\n```\n```\n# Run\n```\n\n```\nnpm dev\n```";
        assert_eq!(
            merge_code_fences(shredded),
            "```\n# Install\nnpm install\n\n# Run\nnpm dev\n```"
        );
    }

    #[test]
    fn merge_code_fences_keeps_prose_between_blocks_separate() {
        // A non-blank, non-fence line ends a run, so distinct blocks with text
        // between them stay distinct.
        let body = "```\nlet a = 1;\n```\n\nSome prose.\n\n```\nlet b = 2;\n```";
        assert_eq!(merge_code_fences(body), body);
    }

    #[test]
    fn merge_code_fences_leaves_fence_free_text_untouched() {
        let body = "# Heading\n\nJust prose, no code.";
        assert_eq!(merge_code_fences(body), body);
    }

    #[test]
    fn converts_html_img_tags_to_markdown() {
        // Self-closing remote image (as seen in older Day One exports).
        assert_eq!(
            recover_html_embeds(r#"a <img src="http://h.local/1.jpeg"/> b"#),
            "a ![](http://h.local/1.jpeg) b"
        );
        // Single quotes, extra attributes, uppercase tag.
        assert_eq!(
            recover_html_embeds(r#"<IMG width='9' src='x.png'>"#),
            "![](x.png)"
        );
        // An img src pointing at a moment stays a moment link for the next step.
        assert_eq!(
            recover_html_embeds(r#"<img src="dayone-moment://ID">"#),
            "![](dayone-moment://ID)"
        );
        // No src → tag dropped; surrounding text preserved.
        assert_eq!(recover_html_embeds("x <img alt='y'> z"), "x  z");
        // Text without tags is unchanged.
        assert_eq!(recover_html_embeds("no tags here"), "no tags here");
    }

    #[test]
    fn recovers_anchor_wrapped_image_as_linked_image() {
        // The shape older Day One entries use for remote image embeds.
        assert_eq!(
            recover_html_embeds(
                r#"><a href="http://500px.com/photo/50215636"><img src="http://h/1.jpeg" alt="Caras"/></a>"#
            ),
            // Leading `>` blockquote marker is preserved.
            ">[![](http://h/1.jpeg)](http://500px.com/photo/50215636)"
        );
    }

    #[test]
    fn recovers_plain_anchor_as_markdown_link() {
        assert_eq!(
            recover_html_embeds(r#"see <a href="https://x.test">the site</a> now"#),
            "see [the site](https://x.test) now"
        );
    }

    #[test]
    fn recovers_multiline_audio_and_video_as_links() {
        let audio = "before\n<audio controls>\n  <source src=\"http://h/1.mp3\" type=\"audio/mpeg\">\n</audio>\nafter";
        assert_eq!(
            recover_html_embeds(audio),
            "before\n[audio](http://h/1.mp3)\nafter"
        );
        assert_eq!(
            recover_html_embeds("<video><source src=\"http://h/clip.mp4\"></video>"),
            "[video](http://h/clip.mp4)"
        );
    }

    #[test]
    fn does_not_convert_html_inside_code_blocks() {
        let body = "before\n```\n<img src=\"x.png\">\n```\n<img src=\"y.png\">";
        assert_eq!(
            recover_html_embeds(body),
            "before\n```\n<img src=\"x.png\">\n```\n![](y.png)"
        );
    }
}
