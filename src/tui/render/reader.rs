use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use notema_domain::Entry;
use std::path::Path;

use crate::tui::{
    app::{App, Focus, ReaderHeading, ReaderImageHits, ReaderLinkHit, RenderedEntryBody},
    image::{digit_for_image, sole_image_ref},
    render::{
        count_label, entry_metadata_layout, panel_block, render_centered_notice,
        render_scrollbar_if_needed, viewer_scroll,
    },
    state::HoverTarget,
    surface::{EntryMetadataValues, PanelGeometry, metadata_section_height},
    theme::theme,
};

use super::markdown::render_text_chunk;
use super::metadata::{EntryMetadata, draw_metadata_section, metadata_section_lines};

/// The body (writing/reading area) is kept at least this tall; the metadata block
/// only pins below the body when the pane can still afford these lines, otherwise it
/// folds into the scroll.
const MIN_ENTRY_BODY_LINES: u16 = 20;

pub(crate) fn draw_selected_reader(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    if let Some((title, content)) = app.selected_reader() {
        let metadata = app
            .resolved_selected_entry()
            .map(Entry::metadata_bundle)
            .unwrap_or_default();
        let entry_path = app.selected_entry_target().map(|target| target.path);

        let (scroll, hits, content_rect, line_count) = draw_markdown_panel(
            frame,
            area,
            app,
            PanelEntry {
                title: &title,
                content: &content,
                word_count: app.selected_entry_word_count(),
                metadata: EntryMetadata::from_metadata(&metadata),
            },
            app.nav.scroll.reader,
            app.nav.focus == Focus::Reader,
            entry_path.as_deref(),
        );
        app.nav.scroll.reader = scroll;
        app.reader_image_hits = ReaderImageHits {
            content_rect,
            scroll,
            line_count,
            labels: hits.images,
            links: hits.links,
            headings: hits.headings,
        };
    } else {
        let block = panel_block("Entry", app.nav.focus == Focus::Reader, None);
        let content = PanelGeometry::new(area).content;
        frame.render_widget(block, area);
        super::panel_focus_stripe(frame, area, app.nav.focus == Focus::Reader);
        render_centered_notice(frame, content, "No entry selected");
    }
}

/// The entry content rendered by the markdown panel.
struct PanelEntry<'a> {
    title: &'a str,
    content: &'a str,
    /// Precomputed on the entry, so the panel title never re-tokenizes the body.
    word_count: usize,
    metadata: EntryMetadata<'a>,
}

/// Draw the entry body and metadata, returning the applied scroll, the clickable
/// image-label positions (`(body line index, image index)`), the body rect (for
/// mapping clicks back to labels), and the total rendered line count (for scrollbar
/// drag mapping).
fn draw_markdown_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    entry: PanelEntry<'_>,
    requested_scroll: u16,
    focused: bool,
    entry_path: Option<&Path>,
) -> (u16, RenderedEntryBody, Rect, usize) {
    let PanelEntry {
        title,
        content,
        word_count,
        metadata,
    } = entry;
    let block = panel_block(
        title,
        focused,
        Some(count_label(word_count, "word", "words")),
    );
    let layout = entry_metadata_layout(area, metadata.values());
    let metadata_scrolls = metadata_scrolls_with_body(area, metadata.values());
    let content_rect = if metadata_scrolls {
        PanelGeometry::new(area).content
    } else {
        layout.content
    };
    // The metadata (when it scrolls with the body) shares the paragraph, so keep
    // the full width there; otherwise gutter the body to a readable max width.
    let body_rect = if metadata_scrolls {
        content_rect
    } else {
        centered_body_rect(content_rect, app.config.ui.layout.reader.body_max_width)
    };

    let width = body_rect.width.saturating_sub(1).max(1) as usize;
    // Memoized on (entry path, width, data version): the markdown parse + syntax
    // highlight + render is the reader's dominant per-frame cost, so a frame that
    // only scrolled, blinked, or ticked images reuses the rendered lines.
    let show_link_urls = app.config.ui.layout.reader.show_link_urls;
    let body = app.cached_entry_body(entry_path, width, || {
        build_body_lines(content, width, entry_path, show_link_urls)
    });
    let mut lines = body.lines.clone();
    if let Some(flash) = app.reader_anchor_flash.as_ref()
        && flash.until > std::time::Instant::now()
        && let Some(line) = lines.get_mut(flash.line)
    {
        *line = line
            .clone()
            .patch_style(Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD));
    }
    // A hovered link/label reverses its own ink into a solid highlight — the
    // app's strong-highlight idiom (see the anchor flash), unmistakable under
    // the cursor on every theme and chrome, using each theme's link color with
    // no per-theme tuning.
    let hovered_link = Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD);
    match app.hover {
        HoverTarget::ReaderImage(line) => {
            if let Some(line) = lines.get_mut(line) {
                *line = line.clone().patch_style(hovered_link);
            }
        }
        HoverTarget::ReaderLink { line, start, end } => {
            // A wrapped link name is several hit segments sharing one group;
            // highlight every segment so the whole name inverts as one link
            // rather than only the row under the cursor.
            let group = body
                .links
                .iter()
                .find(|hit| hit.line == line && hit.start == start && hit.end == end)
                .map(|hit| hit.group);
            if let Some(group) = group {
                for hit in body.links.iter().filter(|hit| hit.group == group) {
                    if let Some(line) = lines.get_mut(hit.line) {
                        patch_line_range(line, hit.start, hit.end, hovered_link);
                    }
                }
            }
        }
        _ => {}
    }
    let hits = RenderedEntryBody {
        lines: Vec::new(),
        images: body.images.clone(),
        links: body.links.clone(),
        headings: body.headings.clone(),
    };
    if metadata_scrolls {
        let meta_lines = metadata_section_lines(body_rect.width, &metadata);
        if !meta_lines.is_empty() {
            let height = body_rect.height as usize;
            if lines.len() + meta_lines.len() < height {
                // Fits: bottom-attach the metadata to the pane's bottom edge,
                // matching the pinned layout on taller panes.
                lines.resize(height - meta_lines.len(), Line::from(""));
            } else {
                // Overflows: one blank line sets the metadata off from the body as
                // it scrolls into view.
                lines.push(Line::from(""));
            }
            lines.extend(meta_lines);
        }
    }
    let line_count = lines.len();
    let scroll = viewer_scroll(requested_scroll, line_count, body_rect.height);
    // Float a short entry in the vertical middle — but only when the metadata is
    // pinned (taller panes). On short panes the body flows from the top with the
    // metadata bottom-attached, so centering would fight that.
    let body_rect = if app.config.ui.layout.reader.body_center_vertically && !metadata_scrolls {
        center_body_vertically(body_rect, line_count, scroll)
    } else {
        body_rect
    };

    frame.render_widget(block, area);
    super::panel_focus_stripe(frame, area, focused);
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme().text())
            .scroll((scroll, 0)),
        body_rect,
    );

    if !metadata_scrolls && layout.metadata.is_some() {
        draw_metadata_section(frame, layout, &metadata);
    }

    render_scrollbar_if_needed(
        frame,
        area,
        line_count,
        body_rect.height,
        scroll as usize,
        focused,
    );

    (scroll, hits, body_rect, line_count)
}

/// Pin the metadata below the body only when doing so still leaves the body at least
/// [`MIN_ENTRY_BODY_LINES`]; otherwise fold it into the scroll. With no metadata the
/// height is zero and this reduces to a plain minimum-body check.
pub(crate) fn metadata_scrolls_with_body(area: Rect, values: EntryMetadataValues<'_>) -> bool {
    let inner = PanelGeometry::new(area).content;
    let metadata_height = metadata_section_height(inner.width, values);
    inner.height < MIN_ENTRY_BODY_LINES.saturating_add(metadata_height)
}

/// Cap `rect` at `max_width` and center it horizontally, leaving the height and
/// narrower panels untouched. A `max_width` of 0 means no cap.
fn centered_body_rect(rect: Rect, max_width: u16) -> Rect {
    if max_width == 0 || rect.width <= max_width {
        return rect;
    }
    let x = rect.x + (rect.width - max_width) / 2;
    Rect {
        x,
        width: max_width,
        ..rect
    }
}

/// When the entry fits without scrolling, push it down so it sits in the vertical
/// middle of `rect`; otherwise render from the top so scrolling covers every line.
fn center_body_vertically(rect: Rect, line_count: usize, scroll: u16) -> Rect {
    if scroll > 0 || line_count >= rect.height as usize {
        return rect;
    }
    let pad = (rect.height - line_count as u16) / 2;
    Rect {
        y: rect.y + pad,
        height: rect.height - pad,
        ..rect
    }
}

/// Build the entry-body lines, replacing each lone in-folder image with a
/// clickable `[Image N …]` label and recording `(body line index, image index)`
/// so clicks and the viewer agree on numbering. Without an entry path the body
/// is rendered as-is.
fn build_body_lines(
    content: &str,
    width: usize,
    entry_path: Option<&Path>,
    show_urls: bool,
) -> RenderedEntryBody {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut links: Vec<ReaderLinkHit> = Vec::new();
    let mut headings: Vec<ReaderHeading> = Vec::new();
    // Offsets each chunk's link `group` into a document-unique range so grouping
    // hover highlights never merge links from different chunks.
    let mut group_base = 0usize;

    let Some(entry_path) = entry_path else {
        lines.push(Line::from(""));
        append_chunk(
            &mut lines,
            &mut links,
            &mut headings,
            &mut group_base,
            render_text_chunk(content, width, show_urls),
        );
        dedupe_heading_anchors(&mut headings);
        return RenderedEntryBody {
            lines,
            links,
            headings,
            ..RenderedEntryBody::default()
        };
    };

    // A leading blank row so the body starts one line below the border, matching
    // the blank that leads the journal and entry columns.
    lines.push(Line::from(""));
    let mut labels = Vec::new();
    let mut buffer = String::new();
    let mut image_index = 0usize;
    // True while the last emitted row was an image label with nothing buffered
    // since. Lets a blank source line right after an image emit an explicit blank
    // row instead of being swallowed by the empty buffer, preserving the gap.
    let mut after_image = false;

    for line in content.split('\n') {
        let Some((alt, _asset)) = sole_image_ref(line, entry_path) else {
            if after_image && buffer.is_empty() && line.trim().is_empty() {
                lines.push(Line::from(""));
                continue;
            }
            if !buffer.is_empty() {
                buffer.push('\n');
            }
            buffer.push_str(line);
            after_image = false;
            continue;
        };

        if !buffer.is_empty() {
            // A trailing blank source line leaves the buffer ending in a single
            // `\n`, which the renderer collapses; add a second so the gap before
            // the label survives.
            if buffer.ends_with('\n') {
                buffer.push('\n');
            }
            append_chunk(
                &mut lines,
                &mut links,
                &mut headings,
                &mut group_base,
                render_text_chunk(&buffer, width, show_urls),
            );
            buffer.clear();
        }
        after_image = true;

        let start_row = lines.len();
        lines.push(image_label_line(image_index, &alt));
        labels.push((start_row, image_index));
        image_index += 1;
    }

    if !buffer.is_empty() {
        append_chunk(
            &mut lines,
            &mut links,
            &mut headings,
            &mut group_base,
            render_text_chunk(&buffer, width, show_urls),
        );
    }

    dedupe_heading_anchors(&mut headings);
    RenderedEntryBody {
        lines,
        images: labels,
        links,
        headings,
    }
}

/// Append a rendered chunk, shifting its link/heading line indices (chunk-local)
/// to their position in the assembled body.
fn append_chunk(
    lines: &mut Vec<Line<'static>>,
    links: &mut Vec<ReaderLinkHit>,
    headings: &mut Vec<ReaderHeading>,
    group_base: &mut usize,
    chunk: super::markdown::RenderedChunk,
) {
    let base = lines.len();
    let group_offset = *group_base;
    links.extend(chunk.links.into_iter().map(|mut hit| {
        hit.line += base;
        hit.group += group_offset;
        hit
    }));
    headings.extend(chunk.headings.into_iter().map(|mut heading| {
        heading.line += base;
        heading
    }));
    lines.extend(chunk.lines);
    *group_base += chunk.link_count;
}

/// Disambiguate repeated heading anchors across the whole document the way the
/// renderer cannot per chunk: the second `intro` becomes `intro-1`, the third
/// `intro-2`, matching in-page anchor links.
fn dedupe_heading_anchors(headings: &mut [ReaderHeading]) {
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for heading in headings.iter_mut() {
        let count = counts.entry(heading.anchor.clone()).or_default();
        if *count > 0 {
            heading.anchor = format!("{}-{count}", heading.anchor);
        }
        *count += 1;
    }
}

/// Patch `style` onto the spans of `line` fully inside the `[start, end)` column
/// range — the seam that lifts a hovered link name. The range comes from the
/// name span's own column/width, so span boundaries align with it.
fn patch_line_range(line: &mut Line<'static>, start: usize, end: usize, style: Style) {
    let mut column = 0usize;
    for span in &mut line.spans {
        let span_end = column.saturating_add(span.width());
        if column >= start && span_end <= end {
            span.style = span.style.patch(style);
        }
        column = span_end;
    }
}

/// A clickable `[Image N: alt - click here or press K]` label. The number is
/// 1-based; images 1-9 bind to their digit, the tenth to `0`, and later images
/// drop the `press K` hint (no digit left to bind).
fn image_label_line(index: usize, alt: &str) -> Line<'static> {
    let alt = alt.trim();
    let number = index + 1;
    let head = if alt.is_empty() {
        format!("Image {number}")
    } else {
        format!("Image {number}: {alt}")
    };
    let text = match digit_for_image(index) {
        Some(key) => format!("[{head} - click here or press {key}]"),
        None => format!("[{head} - click here]"),
    };
    Line::from(Span::styled(text, theme().md_link()))
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn entry_path_with_asset() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let assets = dir.path().join("2026-07-05T14-30-00-abc123.assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("x9k2.png"), b"img").unwrap();
        fs::write(assets.join("aa11.png"), b"img").unwrap();
        let entry_path = dir.path().join("2026-07-05T14-30-00-abc123.md");
        fs::write(&entry_path, b"entry").unwrap();
        (dir, entry_path)
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn image_label_includes_alt_and_press_hint_and_is_one_based() {
        assert_eq!(
            line_text(&image_label_line(0, "sunset")),
            "[Image 1: sunset - click here or press 1]"
        );
        assert_eq!(
            line_text(&image_label_line(3, "")),
            "[Image 4 - click here or press 4]"
        );
    }

    #[test]
    fn tenth_image_binds_to_zero_key() {
        assert_eq!(
            line_text(&image_label_line(9, "")),
            "[Image 10 - click here or press 0]"
        );
    }

    #[test]
    fn image_label_past_ten_drops_press_hint() {
        assert_eq!(
            line_text(&image_label_line(10, "late")),
            "[Image 11: late - click here]"
        );
    }

    /// Each lone in-folder image becomes a numbered clickable label, and its body
    /// line index is recorded so clicks map back to the right image.
    #[test]
    fn replaces_images_with_numbered_labels_and_records_positions() {
        let (_guard, entry_path) = entry_path_with_asset();
        let content = concat!(
            "Text above\n",
            "\n",
            "![a shot](2026-07-05T14-30-00-abc123.assets/x9k2.png)\n",
            "\n",
            "![](2026-07-05T14-30-00-abc123.assets/aa11.png)\n",
            "\n",
            "Text below",
        );

        let body = build_body_lines(content, 40, Some(&entry_path), true);

        let rendered: Vec<String> = body.lines.iter().map(line_text).collect();
        assert_eq!(
            rendered,
            vec![
                String::new(),
                "Text above".to_string(),
                "[Image 1: a shot - click here or press 1]".to_string(),
                String::new(),
                "[Image 2 - click here or press 2]".to_string(),
                String::new(),
                "Text below".to_string(),
            ],
        );
        assert_eq!(body.images, vec![(2, 0), (4, 1)]);
    }

    /// Without an entry path (no selected entry) the body renders untouched.
    #[test]
    fn no_labels_without_entry_path() {
        let body = build_body_lines("just text", 40, None, true);
        assert!(body.images.is_empty());
    }

    #[test]
    fn renderer_records_heading_anchors_and_link_cells() {
        let body = build_body_lines("# My Heading\n\n[Jump](#my-heading)", 40, None, true);

        assert_eq!(
            body.headings,
            [ReaderHeading {
                anchor: "my-heading".to_string(),
                line: 1,
            }]
        );
        assert_eq!(body.links.len(), 1);
        assert_eq!(body.links[0].target, "#my-heading");
        // The clickable region is the name; the target trails it in the faint
        // secondary style.
        let link_line = &body.lines[body.links[0].line];
        assert_eq!(link_line.spans[0].content, "Jump");
        assert_eq!(link_line.spans[0].style, theme().md_link());
        assert_eq!(link_line.spans.last().unwrap().content, " (#my-heading)");
        assert_eq!(link_line.spans.last().unwrap().style, theme().muted());
        assert_eq!((body.links[0].start, body.links[0].end), (0, 4));
    }

    /// A bare autolink renders once (no redundant parenthetical) and stays
    /// clickable over its own text.
    #[test]
    fn autolink_renders_once_and_stays_clickable() {
        let body = build_body_lines("<https://example.com>", 60, None, true);

        assert_eq!(body.links.len(), 1);
        assert_eq!(body.links[0].target, "https://example.com");
        let link_line = &body.lines[body.links[0].line];
        assert_eq!(line_text(link_line), "https://example.com");
    }

    /// With link URLs hidden, the faint `(url)` trailer is stripped from the
    /// display but the name stays clickable over the same columns.
    #[test]
    fn hidden_link_urls_strip_the_trailer_but_keep_the_link() {
        let body = build_body_lines("See [the docs](https://example.com) now.", 60, None, false);

        let link_line = &body.lines[body.links[0].line];
        assert_eq!(line_text(link_line), "See the docs now.");
        assert_eq!(body.links.len(), 1);
        assert_eq!(body.links[0].target, "https://example.com");
        // "See " is 4 cells, "the docs" is 8 — the hit still covers the name.
        assert_eq!((body.links[0].start, body.links[0].end), (4, 12));

        // Shown, the same source keeps the faint trailer.
        let shown = build_body_lines("See [the docs](https://example.com) now.", 60, None, true);
        assert!(line_text(&shown.lines[shown.links[0].line]).contains("(https://example.com)"));
    }

    /// Six consecutive links whose names and ` (url)` trailers straddle wrap
    /// boundaries are all detected — the regression that lost every other link
    /// when semantics were re-scanned per display line.
    #[test]
    fn every_wrapped_consecutive_link_is_detected() {
        let source = "including [Setext](http://docutils.sourceforge.net/mirror/setext.html), \
[atx](http://www.aaronsw.com/2002/atx/), [Textile](http://textism.com/tools/textile/), \
[reStructuredText](http://docutils.sourceforge.net/rst.html), \
[Grutatext](http://www.triptico.com/software/grutatxt.html), \
and [EtText](http://ettext.taint.org/doc/) -- the end.";
        let expected = [
            "http://docutils.sourceforge.net/mirror/setext.html",
            "http://www.aaronsw.com/2002/atx/",
            "http://textism.com/tools/textile/",
            "http://docutils.sourceforge.net/rst.html",
            "http://www.triptico.com/software/grutatxt.html",
            "http://ettext.taint.org/doc/",
        ];

        let shown = build_body_lines(source, 80, None, true);
        let targets: Vec<&str> = shown.links.iter().map(|hit| hit.target.as_str()).collect();
        assert_eq!(targets, expected);
        for hit in &shown.links {
            assert!(hit.end > hit.start);
            assert!(hit.line < shown.lines.len());
        }

        // Hidden URLs: still all six, wrap now computed against the shorter text,
        // and no URL leaks into any rendered line.
        let hidden = build_body_lines(source, 80, None, false);
        let hidden_targets: Vec<&str> =
            hidden.links.iter().map(|hit| hit.target.as_str()).collect();
        assert_eq!(hidden_targets, expected);
        assert!(
            !hidden
                .lines
                .iter()
                .any(|line| line_text(line).contains("http://"))
        );
    }

    /// A link name that itself wraps records one clickable segment per display
    /// line it occupies, each covering a non-empty column span.
    #[test]
    fn a_wrapping_link_name_is_clickable_on_every_row() {
        let body = build_body_lines("[the quick brown fox](https://example.com)", 10, None, false);

        assert!(body.links.len() >= 2);
        // Every segment belongs to the same link, so hovering any row highlights
        // the whole name rather than making it look like several links.
        let group = body.links[0].group;
        for hit in &body.links {
            assert_eq!(hit.target, "https://example.com");
            assert_eq!(hit.group, group);
            assert!(hit.end > hit.start);
        }
    }

    /// Distinct links keep distinct groups, so hovering one never highlights the
    /// other even when they share a target.
    #[test]
    fn distinct_links_have_distinct_groups() {
        let body = build_body_lines(
            "[one](https://example.com) and [two](https://example.com)",
            80,
            None,
            true,
        );

        assert_eq!(body.links.len(), 2);
        assert_ne!(body.links[0].group, body.links[1].group);
    }

    /// A heading that wraps still slugs its whole title (not just the first
    /// display line) and anchors it to that first line.
    #[test]
    fn wrapping_heading_keeps_the_full_anchor_slug() {
        let body = build_body_lines(
            "## A Very Long Heading That Certainly Wraps Across Rows",
            20,
            None,
            true,
        );

        assert_eq!(
            body.headings,
            [ReaderHeading {
                anchor: "a-very-long-heading-that-certainly-wraps-across-rows".to_string(),
                line: 1,
            }]
        );
    }

    /// A relative link target stays styled but is not clickable.
    #[test]
    fn relative_links_are_styled_but_not_clickable() {
        let body = build_body_lines("See [the pic](photo.png) here.", 60, None, true);

        assert!(body.links.is_empty());
        let styled = body
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.content == "the pic" && span.style == theme().md_link());
        assert!(styled);
    }
}
