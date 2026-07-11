//! Minimal inline Markdown scanning shared across the app: the one parser for
//! `[text](target)` / `![text](target)` used by asset ingestion, Day One moment
//! rewriting, and preview redaction.

use std::ops::Range;

/// A parsed inline `[text](target)` or `![text](target)`, with byte ranges into
/// the string it was parsed from. Targets may not contain a `)`.
pub struct InlineSpan {
    pub is_image: bool,
    /// The whole construct, including a leading `!` for images.
    pub span: Range<usize>,
    /// The `text`/`alt` inside the brackets.
    pub text: Range<usize>,
    /// The `target` inside the parentheses.
    pub target: Range<usize>,
}

/// Parse an inline link/image whose marker begins at `s[0]` (`[`, or `!` for an
/// image). Returns `None` unless `s` starts a well-formed `[text](target)` /
/// `![text](target)` with no nested `]` in the text or `)` in the target.
pub fn parse_inline_at(s: &str) -> Option<InlineSpan> {
    let (is_image, text_start) = if s.starts_with("![") {
        (true, 2)
    } else if s.starts_with('[') {
        (false, 1)
    } else {
        return None;
    };

    let text_end = text_start + s[text_start..].find(']')?;
    let paren = text_end + 1;
    if !s[paren..].starts_with('(') {
        return None;
    }
    let target_start = paren + 1;
    let target_end = target_start + s[target_start..].find(')')?;

    Some(InlineSpan {
        is_image,
        span: 0..target_end + 1,
        text: text_start..text_end,
        target: target_start..target_end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_image_span_and_ranges() {
        let span = parse_inline_at("![a cat](cat.png) rest").unwrap();
        assert!(span.is_image);
        assert_eq!(
            &"![a cat](cat.png) rest"[span.span.clone()],
            "![a cat](cat.png)"
        );
        assert_eq!(&"![a cat](cat.png) rest"[span.text], "a cat");
        assert_eq!(&"![a cat](cat.png) rest"[span.target], "cat.png");
    }

    #[test]
    fn parses_plain_link() {
        let span = parse_inline_at("[docs](https://x/y)").unwrap();
        assert!(!span.is_image);
        assert_eq!(&"[docs](https://x/y)"[span.target], "https://x/y");
    }

    #[test]
    fn rejects_bracket_without_parenthesized_target() {
        assert!(parse_inline_at("[just brackets] then").is_none());
        assert!(parse_inline_at("![no target]").is_none());
        assert!(parse_inline_at("plain text").is_none());
    }
}
