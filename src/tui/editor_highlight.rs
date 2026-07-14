//! Light markdown syntax highlighting for the entry editor.
//!
//! Unlike the reader (`render/markdown.rs`), this does not render or reflow: it
//! only *colors* the text in place while editing, keeping every markup character
//! visible. It reuses the `pulldown-cmark` parser — via `into_offset_iter`, which
//! tags each event with its source byte range — so tricky cases like `snake_case`
//! and `a * b * c` are handled correctly, not by ad-hoc scanning.
//!
//! The output is one entry per body line, each a list of `(start, end, style)`
//! byte ranges relative to that line, ready for `TextArea::set_syntax_spans`. The
//! textarea overlays cursor/selection on top, so this is purely a base layer.

use std::ops::Range;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag};
use ratatui::style::{Modifier, Style};

use crate::tui::theme::theme;

/// Compute per-line syntax styling for the whole body. The returned outer `Vec`
/// has one entry per `text.split('\n')` line (matching `TextArea::lines`), each a
/// list of non-overlapping, ordered `(start_byte, end_byte, style)` ranges
/// relative to that line. Bytes with no entry keep the editor's base styling.
pub(crate) fn highlight_body(text: &str) -> Vec<Vec<(usize, usize, Style)>> {
    let theme = theme();
    let muted = theme.muted();
    let md_heading = theme.md_heading();
    let md_heading2 = theme.md_heading2();
    let md_subheading = theme.md_subheading();
    let md_link = theme.md_link();
    let md_code = theme.md_code();
    let md_inline_code = theme.md_inline_code();
    let md_highlight = theme.md_highlight();
    let md_blockquote = theme.md_blockquote();

    // Per-byte style map. `None` leaves the byte to the editor's base styling.
    let mut paint: Vec<Option<Style>> = vec![None; text.len()];

    // Content-style stack. Leaves (`Text`/`Code`) paint with the top style when
    // `meaningful`; markers are filled afterwards from the gaps.
    struct Frame {
        style: Style,
        meaningful: bool,
        span: Range<usize>,
        marker: Marker,
    }
    // How a tag's markup characters are recovered when the tag closes.
    enum Marker {
        /// Delimiters are the non-content gaps in the span (e.g. `**`, `#`, `>`).
        Gap,
        /// Handled at open time (list bullet); nothing to do on close.
        None,
    }

    let base = Style::default();
    let mut stack: Vec<Frame> = vec![Frame {
        style: base,
        meaningful: false,
        span: 0..text.len(),
        marker: Marker::None,
    }];

    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_HEADING_ATTRIBUTES;

    for (event, span) in Parser::new_ext(text, opts).into_offset_iter() {
        let top = stack.last().expect("base frame always present");
        match event {
            Event::Start(tag) => {
                let (style, styling, marker) = match &tag {
                    Tag::Heading { level, .. } => {
                        let s = match level {
                            HeadingLevel::H1 => md_heading,
                            HeadingLevel::H2 => md_heading2,
                            _ => md_subheading,
                        };
                        (s, true, Marker::Gap)
                    }
                    Tag::Strong => (top.style.add_modifier(Modifier::BOLD), true, Marker::Gap),
                    Tag::Emphasis => (top.style.add_modifier(Modifier::ITALIC), true, Marker::Gap),
                    Tag::Strikethrough => (
                        top.style.add_modifier(Modifier::CROSSED_OUT),
                        true,
                        Marker::Gap,
                    ),
                    Tag::Link { .. } => (top.style.patch(md_link), true, Marker::Gap),
                    Tag::BlockQuote(_) => (top.style.patch(md_blockquote), true, Marker::Gap),
                    Tag::CodeBlock(_) => (top.style.patch(md_code), true, Marker::Gap),
                    Tag::Item => {
                        // Paint just the bullet/number marker; the item body stays
                        // untouched (no content style), so `Marker::None`.
                        paint_list_marker(&mut paint, text, span.start, muted);
                        (top.style, false, Marker::None)
                    }
                    // Containers with no markup of their own: inherit the parent.
                    _ => (top.style, false, Marker::None),
                };
                let meaningful = top.meaningful || styling;
                stack.push(Frame {
                    style,
                    meaningful,
                    span: span.clone(),
                    marker,
                });
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let frame = stack.pop().expect("checked len > 1");
                    if let Marker::Gap = frame.marker {
                        fill_markers(&mut paint, text, frame.span, muted);
                    }
                }
            }
            Event::Text(_) => {
                if top.meaningful {
                    fill(&mut paint, span, top.style, true);
                }
            }
            Event::Code(_) => {
                // Inline code is one event including the backtick delimiters. Dim the
                // backtick run on each side and color the inner text as code.
                let ticks = text.as_bytes()[span.clone()]
                    .iter()
                    .take_while(|&&b| b == b'`')
                    .count();
                let inner = (span.start + ticks)..span.end.saturating_sub(ticks);
                fill(&mut paint, span.start..span.start + ticks, muted, true);
                fill(
                    &mut paint,
                    span.end.saturating_sub(ticks)..span.end,
                    muted,
                    true,
                );
                if inner.start < inner.end {
                    fill(&mut paint, inner, top.style.patch(md_inline_code), true);
                }
            }
            Event::TaskListMarker(_) => fill(&mut paint, span, muted, true),
            _ => {}
        }
    }

    // `==highlight==` is not a pulldown extension, so scan for it in still-plain
    // (unpainted) text — consistent with how the reader handles it by hand.
    highlight_marks(&mut paint, text, muted, md_highlight);

    to_line_ranges(text, &paint)
}

/// Paint `range` with `style`; when `overwrite` is false, only paint bytes that are
/// still unset. Out-of-range indices are clamped.
fn fill(paint: &mut [Option<Style>], range: Range<usize>, style: Style, overwrite: bool) {
    let end = range.end.min(paint.len());
    for slot in &mut paint[range.start.min(end)..end] {
        if overwrite || slot.is_none() {
            *slot = Some(style);
        }
    }
}

/// Fill the not-yet-painted, non-whitespace bytes of `range` as markup markers.
/// Content leaves have already claimed their bytes, so what remains is delimiters
/// (`**`, `#`, `>`, code fences, link brackets, …). Markdown delimiters are ASCII,
/// so byte-level handling can't split a multibyte character.
fn fill_markers(paint: &mut [Option<Style>], text: &str, range: Range<usize>, style: Style) {
    let bytes = text.as_bytes();
    let end = range.end.min(paint.len());
    for i in range.start.min(end)..end {
        if paint[i].is_none() && !bytes[i].is_ascii_whitespace() {
            paint[i] = Some(style);
        }
    }
}

/// Paint a list item's leading bullet/number marker (e.g. `- `, `* `, `1. `).
fn paint_list_marker(paint: &mut [Option<Style>], text: &str, start: usize, style: Style) {
    let bytes = text.as_bytes();
    let mut i = start;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    let content_start = i;
    if i < bytes.len() && matches!(bytes[i], b'-' | b'*' | b'+') {
        i += 1;
    } else {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i < bytes.len() && matches!(bytes[i], b'.' | b')') {
            i += 1;
        } else {
            return; // not a recognizable marker
        }
    }
    // Include one trailing space so the marker reads as a unit.
    if i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    fill(paint, content_start..i, style, false);
}

/// Paint `==highlight==` spans that sit entirely in still-plain text on one line:
/// the `==` delimiters muted, the inner text in the theme's highlight style
/// (matching the reader's `==`).
fn highlight_marks(paint: &mut [Option<Style>], text: &str, muted: Style, highlight: Style) {
    let b = text.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'=' && b[i + 1] == b'=' {
            let open = i;
            let mut j = i + 2;
            let mut close = None;
            while j + 1 < b.len() && b[j] != b'\n' {
                if b[j] == b'=' && b[j + 1] == b'=' {
                    close = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(c) = close {
                let inner = open + 2..c;
                let region = open..c + 2;
                if inner.start < inner.end && region.clone().all(|k| paint[k].is_none()) {
                    fill(paint, open..open + 2, muted, true);
                    fill(paint, inner, highlight, true);
                    fill(paint, c..c + 2, muted, true);
                    i = c + 2;
                    continue;
                }
            }
        }
        i += 1;
    }
}

/// Split the per-byte paint map into per-line ranges (offsets relative to each
/// line), coalescing runs of equal style. One entry per `text.split('\n')` line.
fn to_line_ranges(text: &str, paint: &[Option<Style>]) -> Vec<Vec<(usize, usize, Style)>> {
    let mut result = Vec::new();
    let mut line_start = 0usize;
    for line in text.split('\n') {
        let ls = line_start;
        let le = ls + line.len();
        let mut ranges = Vec::new();
        let mut i = ls;
        while i < le {
            if let Some(style) = paint[i] {
                let start = i;
                let mut j = i + 1;
                while j < le && paint[j] == Some(style) {
                    j += 1;
                }
                ranges.push((start - ls, j - ls, style));
                i = j;
            } else {
                i += 1;
            }
        }
        result.push(ranges);
        line_start = le + 1; // skip the '\n'
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_through_widget_no_panic() {
        use ratatui::{Terminal, backend::TestBackend};
        use ratatui_textarea::{TextArea, WrapMode};

        let bodies = [
            "# café ☕ **bold** and 日本語 text\n\nSome `code` and [link](https://例え.jp) here.\n> quote 中文\n- item ✅\n```rust\nlet x = \"日本\";\n```\n==你好== end",
            "***nested café*** with émoji 🚀 that wraps around narrow columns repeatedly",
            "",
        ];
        for width in [8u16, 20, 40] {
            for body in bodies {
                let mut ta = TextArea::new(body.split('\n').map(str::to_string).collect());
                ta.set_wrap_mode(WrapMode::WordOrGlyph);
                ta.set_syntax_spans(highlight_body(body));
                // Move the cursor onto a styled line to mimic editing.
                ta.move_cursor(ratatui_textarea::CursorMove::Bottom);
                let backend = TestBackend::new(width, 12);
                let mut term = Terminal::new(backend).unwrap();
                term.draw(|f| f.render_widget(&ta, f.area())).unwrap();
            }
        }
    }

    #[test]
    fn fuzz_no_panic() {
        let samples = [
            "# café ☕ **bold** and 日本語 text",
            "a `code` `unterminated and ==mark== and 🎉 emoji",
            "**bold with émojis 🚀 inside**",
            "> quote with 中文 and [link](https://例え.jp/パス)",
            "- list ✅ item\n- 二番目 item with `çode`",
            "==你好== and ~~stríke~~",
            "```rust\nlet x = \"日本\";\n```",
            "***nested café***",
            "no markdown at all just prose with é and 🌍",
            "",
            "\n\n\n",
            "*",
            "`",
            "[",
            "](",
            "==",
            "#",
            "> ",
            "text é** unmatched",
        ];
        for s in samples {
            let out = highlight_body(s);
            assert_eq!(out.len(), s.split('\n').count(), "line count for {s:?}");
            // Ranges must be within their line and valid char boundaries.
            for (line, ranges) in s.split('\n').zip(&out) {
                for &(a, b, _) in ranges {
                    assert!(a <= b && b <= line.len(), "oob range {a}..{b} in {line:?}");
                    assert!(
                        line.is_char_boundary(a) && line.is_char_boundary(b),
                        "non-char-boundary {a}..{b} in {line:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn line_count_matches_split() {
        let text = "a\n\nb\nc";
        assert_eq!(highlight_body(text).len(), text.split('\n').count());
    }

    #[test]
    fn bold_dims_markers_and_bolds_content() {
        let muted = theme().muted();
        let ranges = &highlight_body("**bold**")[0];
        // ** (0..2) muted, bold (2..6) bold, ** (6..8) muted.
        assert_eq!(ranges.len(), 3);
        assert_eq!((ranges[0].0, ranges[0].1), (0, 2));
        assert_eq!(ranges[0].2, muted);
        assert_eq!((ranges[1].0, ranges[1].1), (2, 6));
        assert!(ranges[1].2.add_modifier.contains(Modifier::BOLD));
        assert_eq!((ranges[2].0, ranges[2].1), (6, 8));
        assert_eq!(ranges[2].2, muted);
    }

    #[test]
    fn heading_marker_is_dimmed() {
        let muted = theme().muted();
        let ranges = &highlight_body("## Title")[0];
        // "##" dimmed; the space stays unstyled; "Title" is heading-colored.
        assert_eq!(ranges[0].2, muted);
        assert_eq!((ranges[0].0, ranges[0].1), (0, 2));
        assert!(ranges.iter().any(|&(s, e, _)| s == 3 && e == 8));
    }

    #[test]
    fn plain_prose_has_no_ranges() {
        // Emphasis-lookalikes must not be styled.
        for line in highlight_body("snake_case_word and a * b * c math") {
            assert!(line.is_empty(), "unexpected styling: {line:?}");
        }
    }

    #[test]
    fn fenced_code_body_is_colored_across_lines() {
        let md_code = theme().md_code();
        let out = highlight_body("```\nlet x = 1;\nlet y = 2;\n```");
        // Lines 1 and 2 are the code body, colored as code end to end.
        assert!(out[1].iter().any(|&(_, _, st)| st == md_code));
        assert!(out[2].iter().any(|&(_, _, st)| st == md_code));
    }

    #[test]
    fn link_text_colored_brackets_dimmed() {
        let muted = theme().muted();
        let md_link = theme().md_link();
        let ranges = &highlight_body("[hi](https://x)")[0];
        // "hi" (1..3) is link-colored; the surrounding [](url) is dimmed.
        assert!(
            ranges
                .iter()
                .any(|&(s, e, st)| s == 1 && e == 3 && st == md_link)
        );
        assert!(ranges.iter().any(|&(_, _, st)| st == muted));
    }
}
