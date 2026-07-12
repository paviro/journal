use pulldown_cmark::{
    Alignment as MarkdownAlignment, CodeBlockKind, Event as MarkdownEvent, HeadingLevel,
    Options as MarkdownOptions, Parser as MarkdownParser, Tag as MarkdownTag,
    TagEnd as MarkdownTagEnd,
};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::app::{ReaderHeading, ReaderLinkHit};
use crate::tui::theme::theme;

/// The rendered form of one markdown chunk: the wrapped display lines plus the
/// clickable link regions and heading anchors discovered structurally during
/// rendering. Link/heading `line` indices are relative to `lines`.
pub(super) struct RenderedChunk {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) links: Vec<ReaderLinkHit>,
    pub(super) headings: Vec<ReaderHeading>,
    /// Number of link ids minted for this chunk, so the caller can offset the
    /// per-chunk `ReaderLinkHit::group` into a document-unique range.
    pub(super) link_count: usize,
}

/// Render a chunk of markdown text into owned (`'static`) lines. `show_urls`
/// controls whether a link's ` (url)` trailer is emitted; it is applied here,
/// before wrapping, so hidden URLs never skew wrap boundaries.
pub(super) fn render_text_chunk(text: &str, width: usize, show_urls: bool) -> RenderedChunk {
    MarkdownTerminalRenderer::new(width, show_urls).render(text)
}

/// A styled run tagged with the link it belongs to, if any. The renderer
/// accumulates the current logical line as these so link identity survives
/// wrapping; the `link` tag is an index into [`MarkdownTerminalRenderer::link_targets`].
#[derive(Clone)]
struct RichSpan {
    content: String,
    style: Style,
    link: Option<usize>,
}

struct MarkdownTerminalRenderer {
    width: usize,
    show_urls: bool,
    lines: Vec<Line<'static>>,
    current: Vec<RichSpan>,
    styles: Vec<Style>,
    /// The open links/images as `(target, visible text, id)`. The text
    /// accumulates as the link's inner events stream, so the closing tag can drop
    /// the parenthetical when the visible name already is the target (autolinks).
    /// `id` indexes [`Self::link_targets`] and tags the name's characters.
    links: Vec<(String, String, usize)>,
    /// Every link target seen, indexed by the id stored on `RichSpan`s and used
    /// to resolve a recorded hit's URL.
    link_targets: Vec<String>,
    /// Clickable link regions discovered while wrapping, in display-line coords.
    link_hits: Vec<ReaderLinkHit>,
    /// Heading anchors, built from the complete (pre-wrap) heading text.
    headings: Vec<ReaderHeading>,
    /// The visible text of the open heading, accumulated so its anchor slug is
    /// built from the whole title even when the heading wraps across lines.
    heading_text: Option<String>,
    /// Open block containers (blockquotes and lists) in nesting order, so a
    /// blockquote inside a list follows the list's indent and vice versa.
    containers: Vec<Container>,
    code: Option<MarkdownCodeBlock>,
    table: Option<MarkdownTable>,
    separate_next_block: bool,
    highlight_open: bool,
}

enum Container {
    Quote,
    List(MarkdownList),
}

struct MarkdownList {
    next: Option<u64>,
    marker: String,
    first_line: bool,
    in_item: bool,
}

struct MarkdownCodeBlock {
    language: String,
    source: String,
}

struct MarkdownTable {
    alignments: Vec<MarkdownAlignment>,
    rows: Vec<Vec<Line<'static>>>,
    row: Vec<Line<'static>>,
    cell: Line<'static>,
    in_head: bool,
}

impl MarkdownTerminalRenderer {
    fn new(width: usize, show_urls: bool) -> Self {
        Self {
            width: width.max(1),
            show_urls,
            lines: Vec::new(),
            current: Vec::new(),
            styles: vec![Style::default()],
            links: Vec::new(),
            link_targets: Vec::new(),
            link_hits: Vec::new(),
            headings: Vec::new(),
            heading_text: None,
            containers: Vec::new(),
            code: None,
            table: None,
            separate_next_block: false,
            highlight_open: false,
        }
    }

    fn render(mut self, source: &str) -> RenderedChunk {
        let options = MarkdownOptions::ENABLE_TABLES
            | MarkdownOptions::ENABLE_STRIKETHROUGH
            | MarkdownOptions::ENABLE_TASKLISTS
            | MarkdownOptions::ENABLE_GFM
            | MarkdownOptions::ENABLE_HEADING_ATTRIBUTES;
        for event in MarkdownParser::new_ext(source, options) {
            self.event(event);
        }
        self.finish_current(false);
        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }
        RenderedChunk {
            lines: self.lines,
            links: self.link_hits,
            headings: self.headings,
            link_count: self.link_targets.len(),
        }
    }

    fn event(&mut self, event: MarkdownEvent<'_>) {
        if self.code.is_some() {
            match event {
                MarkdownEvent::End(MarkdownTagEnd::CodeBlock) => self.finish_code_block(),
                MarkdownEvent::Text(text)
                | MarkdownEvent::Code(text)
                | MarkdownEvent::Html(text)
                | MarkdownEvent::InlineHtml(text) => {
                    if let Some(code) = self.code.as_mut() {
                        code.source.push_str(&text);
                    }
                }
                MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => {
                    if let Some(code) = self.code.as_mut() {
                        code.source.push('\n');
                    }
                }
                _ => {}
            }
            return;
        }

        if self.table.is_some() && self.table_event(&event) {
            return;
        }

        match event {
            MarkdownEvent::Start(tag) => self.start_tag(tag),
            MarkdownEvent::End(tag) => self.end_tag(tag),
            MarkdownEvent::Text(text) => {
                self.capture_link_text(&text);
                self.capture_heading_text(&text);
                self.push_highlighted_text(&text);
            }
            MarkdownEvent::Code(code) => {
                self.capture_link_text(&code);
                self.capture_heading_text(&code);
                self.push_span(&code, theme().md_code());
            }
            MarkdownEvent::Html(html) | MarkdownEvent::InlineHtml(html) => {
                self.push_multiline(&html, theme().muted());
            }
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => self.finish_current(true),
            MarkdownEvent::Rule => self.render_rule(),
            MarkdownEvent::TaskListMarker(checked) => {
                self.push_span(if checked { "[x] " } else { "[ ] " }, theme().muted())
            }
            MarkdownEvent::FootnoteReference(label) => {
                self.push_span(&format!("[{label}]"), theme().md_link());
            }
            MarkdownEvent::InlineMath(math) => self.push_span(&math, theme().md_code()),
            MarkdownEvent::DisplayMath(math) => {
                self.begin_block();
                self.push_multiline(&math, theme().md_code());
                self.finish_current(false);
                self.separate_next_block = true;
            }
        }
    }

    fn start_tag(&mut self, tag: MarkdownTag<'_>) {
        match tag {
            MarkdownTag::Paragraph => self.start_paragraph(),
            MarkdownTag::Heading { level, .. } => {
                self.begin_block();
                self.heading_text = Some(String::new());
                let style = if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                    theme().md_heading()
                } else {
                    theme().md_heading3()
                };
                self.push_style(style);
                self.push_span(&format!("{} ", "#".repeat(level as usize)), style);
            }
            MarkdownTag::BlockQuote(_) => {
                self.begin_block();
                self.containers.push(Container::Quote);
            }
            MarkdownTag::CodeBlock(kind) => {
                self.begin_block();
                let language = match kind {
                    CodeBlockKind::Fenced(language) => language.into_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some(MarkdownCodeBlock {
                    language,
                    source: String::new(),
                });
            }
            MarkdownTag::List(start) => {
                self.begin_block();
                self.containers.push(Container::List(MarkdownList {
                    next: start,
                    marker: String::new(),
                    first_line: false,
                    in_item: false,
                }));
            }
            MarkdownTag::Item => self.start_list_item(),
            MarkdownTag::Table(alignments) => {
                self.begin_block();
                self.table = Some(MarkdownTable {
                    alignments,
                    rows: Vec::new(),
                    row: Vec::new(),
                    cell: Line::default(),
                    in_head: false,
                });
            }
            MarkdownTag::Emphasis => self.push_style(Style::new().italic()),
            MarkdownTag::Strong => self.push_style(Style::new().bold()),
            MarkdownTag::Strikethrough => {
                self.push_style(Style::new().add_modifier(Modifier::CROSSED_OUT));
            }
            MarkdownTag::Link { dest_url, .. } | MarkdownTag::Image { dest_url, .. } => {
                let target = dest_url.into_string();
                let id = self.link_targets.len();
                self.link_targets.push(target.clone());
                self.links.push((target, String::new(), id));
                self.push_style(theme().md_link());
            }
            MarkdownTag::HtmlBlock
            | MarkdownTag::FootnoteDefinition(_)
            | MarkdownTag::DefinitionList
            | MarkdownTag::DefinitionListTitle
            | MarkdownTag::DefinitionListDefinition
            | MarkdownTag::TableHead
            | MarkdownTag::TableRow
            | MarkdownTag::TableCell
            | MarkdownTag::MetadataBlock(_)
            | MarkdownTag::Superscript
            | MarkdownTag::Subscript => {}
        }
    }

    fn end_tag(&mut self, tag: MarkdownTagEnd) {
        match tag {
            MarkdownTagEnd::Paragraph => {
                self.finish_current(false);
                self.separate_next_block = true;
            }
            MarkdownTagEnd::Heading(_) => {
                self.pop_style();
                // Record the anchor against the heading's first display line —
                // `self.lines.len()` is that index because `finish_current` below
                // is what pushes the heading's rows. The slug comes from the whole
                // title, so it stays correct even when the heading wraps.
                if let Some(text) = self.heading_text.take() {
                    let anchor = heading_anchor(text.trim());
                    if !anchor.is_empty() {
                        self.headings.push(ReaderHeading {
                            anchor,
                            line: self.lines.len(),
                        });
                    }
                }
                self.finish_current(false);
                self.separate_next_block = true;
            }
            MarkdownTagEnd::BlockQuote(_) => {
                self.finish_current(false);
                self.containers.pop();
                self.separate_next_block = true;
            }
            MarkdownTagEnd::List(_) => {
                self.finish_current(false);
                self.containers.pop();
                self.separate_next_block = !self.in_list();
            }
            MarkdownTagEnd::Item => {
                self.finish_current(false);
                if let Some(list) = self.current_list_mut() {
                    list.in_item = false;
                }
                self.separate_next_block = false;
            }
            MarkdownTagEnd::Emphasis | MarkdownTagEnd::Strong | MarkdownTagEnd::Strikethrough => {
                self.pop_style()
            }
            MarkdownTagEnd::Link | MarkdownTagEnd::Image => {
                self.pop_style();
                if let Some((target, text, _id)) = self.links.pop()
                    && self.show_urls
                    && text.trim() != target.trim()
                {
                    // The name (tagged with its link id, so it is the clickable
                    // region) stays `md_link`; the untagged target trails it in the
                    // faint secondary style. When URLs are hidden the trailer is
                    // skipped entirely, so wrapping never accounts for it.
                    self.push_span(" (", theme().muted());
                    self.push_span(&target, theme().muted());
                    self.push_span(")", theme().muted());
                }
            }
            MarkdownTagEnd::CodeBlock | MarkdownTagEnd::Table => {}
            MarkdownTagEnd::HtmlBlock
            | MarkdownTagEnd::FootnoteDefinition
            | MarkdownTagEnd::TableHead
            | MarkdownTagEnd::TableRow
            | MarkdownTagEnd::TableCell
            | MarkdownTagEnd::MetadataBlock(_)
            | MarkdownTagEnd::DefinitionList
            | MarkdownTagEnd::DefinitionListTitle
            | MarkdownTagEnd::DefinitionListDefinition
            | MarkdownTagEnd::Superscript
            | MarkdownTagEnd::Subscript => {}
        }
    }

    fn start_paragraph(&mut self) {
        // A blank separator before this block (a second paragraph in the item, or
        // a block after one) is driven by `separate_next_block` through
        // `begin_block`, which runs before any nested container is entered — so the
        // separator sits at the list's indent and never inherits a blockquote rail.
        self.begin_block();
    }

    fn start_list_item(&mut self) {
        self.finish_current(false);
        let Some(list) = self.current_list_mut() else {
            return;
        };
        list.marker = match list.next {
            Some(number) => {
                list.next = Some(number.saturating_add(1));
                format!("{number}. ")
            }
            None => "- ".to_string(),
        };
        list.first_line = true;
        list.in_item = true;
        self.separate_next_block = false;
    }

    fn begin_block(&mut self) {
        self.finish_current(false);
        if self.separate_next_block && !self.lines.is_empty() {
            self.emit_blank_line();
        }
        self.separate_next_block = false;
    }

    fn push_style(&mut self, style: Style) {
        self.styles.push(self.current_style().patch(style));
    }

    fn pop_style(&mut self) {
        if self.styles.len() > 1 {
            self.styles.pop();
        }
    }

    fn current_style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_span(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        if let Some(table) = self.table.as_mut() {
            // Table cells carry no link semantics; keep the plain ratatui merge.
            if let Some(span) = table.cell.spans.last_mut()
                && span.style == style
            {
                span.content.to_mut().push_str(text);
            } else {
                table.cell.spans.push(Span::styled(text.to_string(), style));
            }
            return;
        }
        let link = self.current_link();
        if let Some(span) = self.current.last_mut()
            && span.style == style
            && span.link == link
        {
            span.content.push_str(text);
        } else {
            self.current.push(RichSpan {
                content: text.to_string(),
                style,
                link,
            });
        }
    }

    /// The id of the innermost open link/image, tagging characters emitted while
    /// it is open so wrapping can recover the clickable region.
    fn current_link(&self) -> Option<usize> {
        self.links.last().map(|(_, _, id)| *id)
    }

    /// Accumulate an open link's visible text so the closing tag can compare it
    /// against the target (autolinks render the URL as their own name).
    fn capture_link_text(&mut self, text: &str) {
        if let Some((_, name, _)) = self.links.last_mut() {
            name.push_str(text);
        }
    }

    /// Accumulate the open heading's visible text for its anchor slug.
    fn capture_heading_text(&mut self, text: &str) {
        if let Some(heading) = self.heading_text.as_mut() {
            heading.push_str(text);
        }
    }

    fn push_multiline(&mut self, text: &str, style: Style) {
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.finish_current(true);
            }
            self.push_span(line, style);
        }
    }

    fn push_highlighted_text(&mut self, text: &str) {
        let mut rest = text;
        while let Some(index) = rest.find("==") {
            self.push_span(&rest[..index], self.highlight_style());
            self.highlight_open = !self.highlight_open;
            rest = &rest[index + 2..];
        }
        self.push_span(rest, self.highlight_style());
    }

    fn highlight_style(&self) -> Style {
        if self.highlight_open {
            self.current_style()
                .patch(theme().primary())
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            self.current_style()
        }
    }

    fn finish_current(&mut self, force: bool) {
        if !force && self.current.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.current);
        self.emit_wrapped_line(spans, None, false);
    }

    fn emit_blank_line(&mut self) {
        self.emit_wrapped_line(Vec::new(), None, false);
    }

    /// The innermost open list, ignoring any blockquotes nested inside it.
    fn current_list_mut(&mut self) -> Option<&mut MarkdownList> {
        self.containers
            .iter_mut()
            .rev()
            .find_map(|container| match container {
                Container::List(list) => Some(list),
                Container::Quote => None,
            })
    }

    fn in_list(&self) -> bool {
        self.containers
            .iter()
            .any(|container| matches!(container, Container::List(_)))
    }

    fn container_prefix(&mut self, markers: bool) -> Line<'static> {
        let mut spans = Vec::new();
        for container in &mut self.containers {
            match container {
                Container::Quote => {
                    spans.push(Span::styled("│ ", theme().md_blockquote()));
                }
                Container::List(list) if list.in_item => {
                    if markers && list.first_line {
                        spans.push(Span::styled(list.marker.clone(), theme().muted()));
                        list.first_line = false;
                    } else {
                        spans.push(Span::raw(" ".repeat(list.marker.width())));
                    }
                }
                Container::List(_) => {}
            }
        }
        Line::from(spans)
    }

    fn emit_wrapped_line(
        &mut self,
        spans: Vec<RichSpan>,
        rail: Option<&'static str>,
        hard_wrap: bool,
    ) {
        let mut first_prefix = self.container_prefix(true);
        if let Some(rail) = rail {
            first_prefix
                .spans
                .push(Span::styled(rail, theme().md_blockquote()));
        }
        let available = self.width.saturating_sub(first_prefix.width()).max(1);
        let wrapped = wrap_rich(spans, available, hard_wrap);
        for (index, content) in wrapped.into_iter().enumerate() {
            let mut prefix = if index == 0 {
                first_prefix.clone()
            } else {
                let mut prefix = self.container_prefix(false);
                if let Some(rail) = rail {
                    prefix
                        .spans
                        .push(Span::styled(rail, theme().md_blockquote()));
                }
                prefix
            };
            // A link name that straddles a wrap boundary records one hit segment
            // per display line; the prefix (blockquote rail, list indent) shifts
            // every content column, so hit columns include it — matching the
            // absolute body-line columns click hit-testing compares against.
            let prefix_width = prefix.width();
            for (start, end, id) in content.links {
                let target = &self.link_targets[id];
                if is_openable_link(target) {
                    self.link_hits.push(ReaderLinkHit {
                        line: self.lines.len(),
                        start: prefix_width.saturating_add(start),
                        end: prefix_width.saturating_add(end),
                        target: target.clone(),
                        group: id,
                    });
                }
            }
            prefix.spans.extend(content.spans);
            self.lines.push(prefix);
        }
    }

    fn render_rule(&mut self) {
        self.begin_block();
        let prefix_width = self.container_prefix(false).width();
        let width = self.width.saturating_sub(prefix_width).max(1);
        let glyph = theme().glyphs().borders.line_set().horizontal;
        self.emit_wrapped_line(
            vec![RichSpan {
                content: glyph.repeat(width),
                style: theme().muted(),
                link: None,
            }],
            None,
            true,
        );
        self.separate_next_block = true;
    }

    fn finish_code_block(&mut self) {
        let Some(code) = self.code.take() else {
            return;
        };
        let language = code.language.split_whitespace().next().unwrap_or_default();
        let header = if language.is_empty() {
            "╭─".to_string()
        } else {
            format!("╭─ {language}")
        };
        self.emit_wrapped_line(
            vec![RichSpan {
                content: header,
                style: theme().md_blockquote(),
                link: None,
            }],
            None,
            true,
        );
        let source = code.source.trim_end_matches('\n').replace('\t', "    ");
        let highlighted = crate::tui::syntax_highlight::highlight(language, &source);
        let code_lines = highlighted.unwrap_or_else(|| {
            source
                .split('\n')
                .map(|line| Line::from(Span::styled(line.to_string(), theme().md_code())))
                .collect()
        });
        for line in code_lines {
            self.emit_wrapped_line(rich_from_line(line), Some("│ "), true);
        }
        self.emit_wrapped_line(
            vec![RichSpan {
                content: "╰─".to_string(),
                style: theme().md_blockquote(),
                link: None,
            }],
            None,
            true,
        );
        self.separate_next_block = true;
    }

    fn table_event(&mut self, event: &MarkdownEvent<'_>) -> bool {
        match event {
            MarkdownEvent::Start(MarkdownTag::TableHead) => {
                self.table.as_mut().expect("table exists").in_head = true;
            }
            MarkdownEvent::End(MarkdownTagEnd::TableHead) => {
                let table = self.table.as_mut().expect("table exists");
                table.in_head = false;
                if !table.row.is_empty() {
                    table.rows.push(std::mem::take(&mut table.row));
                }
            }
            MarkdownEvent::Start(MarkdownTag::TableRow) => {}
            MarkdownEvent::End(MarkdownTagEnd::TableRow) => {
                let table = self.table.as_mut().expect("table exists");
                table.rows.push(std::mem::take(&mut table.row));
            }
            MarkdownEvent::Start(MarkdownTag::TableCell) => {}
            MarkdownEvent::End(MarkdownTagEnd::TableCell) => {
                let table = self.table.as_mut().expect("table exists");
                let mut cell = std::mem::take(&mut table.cell);
                if table.in_head {
                    cell = cell.patch_style(Style::new().bold());
                }
                table.row.push(cell);
            }
            MarkdownEvent::End(MarkdownTagEnd::Table) => self.finish_table(),
            MarkdownEvent::Start(MarkdownTag::Emphasis) => self.push_style(Style::new().italic()),
            MarkdownEvent::End(MarkdownTagEnd::Emphasis) => self.pop_style(),
            MarkdownEvent::Start(MarkdownTag::Strong) => self.push_style(Style::new().bold()),
            MarkdownEvent::End(MarkdownTagEnd::Strong) => self.pop_style(),
            MarkdownEvent::Start(MarkdownTag::Strikethrough) => {
                self.push_style(Style::new().add_modifier(Modifier::CROSSED_OUT));
            }
            MarkdownEvent::End(MarkdownTagEnd::Strikethrough) => self.pop_style(),
            MarkdownEvent::Code(code) => self.push_span(code, theme().md_code()),
            MarkdownEvent::Text(text) => self.push_highlighted_text(text),
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => {
                self.push_span(" ", self.current_style());
            }
            _ => {}
        }
        true
    }

    fn finish_table(&mut self) {
        let Some(table_data) = self.table.take() else {
            return;
        };
        if table_data.rows.is_empty() {
            return;
        }
        let columns = table_data
            .rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or_default();
        if columns == 0 {
            return;
        }
        let prefix_width = self.container_prefix(false).width();
        let available = self.width.saturating_sub(prefix_width);
        let overhead = columns.saturating_mul(3).saturating_add(1);
        if available <= overhead.saturating_add(columns) {
            self.render_stacked_table(table_data.rows);
            self.separate_next_block = true;
            return;
        }
        let content_width = available - overhead;
        let mut widths = vec![1usize; columns];
        for row in &table_data.rows {
            for (column, cell) in row.iter().enumerate() {
                widths[column] = widths[column].max(cell.width());
            }
        }
        while widths.iter().sum::<usize>() > content_width {
            let Some((column, _)) = widths.iter().enumerate().max_by_key(|(_, width)| **width)
            else {
                break;
            };
            if widths[column] <= 1 {
                break;
            }
            widths[column] -= 1;
        }

        let border = super::table::border_style();
        self.emit_table_line(super::table::rule(
            &widths,
            super::table::RulePos::Top,
            border,
            border,
        ));
        for (row_index, row) in table_data.rows.iter().enumerate() {
            let wrapped: Vec<Vec<Line<'static>>> = (0..columns)
                .map(|column| {
                    row.get(column).cloned().map_or_else(
                        || vec![Line::default()],
                        |cell| wrap_line(cell, widths[column]),
                    )
                })
                .collect();
            let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
            for line_index in 0..height {
                let mut spans = vec![super::table::border()];
                for column in 0..columns {
                    let mut cell = wrapped[column]
                        .get(line_index)
                        .cloned()
                        .unwrap_or_default()
                        .spans;
                    let used = cell.iter().map(Span::width).sum::<usize>();
                    let padding = widths[column].saturating_sub(used);
                    let (left, right) = match table_data
                        .alignments
                        .get(column)
                        .copied()
                        .unwrap_or(MarkdownAlignment::None)
                    {
                        MarkdownAlignment::Right => (padding, 0),
                        MarkdownAlignment::Center => (padding / 2, padding - padding / 2),
                        MarkdownAlignment::None | MarkdownAlignment::Left => (0, padding),
                    };
                    if left > 0 {
                        cell.insert(0, Span::raw(" ".repeat(left)));
                    }
                    if right > 0 {
                        cell.push(Span::raw(" ".repeat(right)));
                    }
                    super::table::push_cell_spans(&mut spans, cell);
                }
                self.emit_table_line(Line::from(spans));
            }
            if row_index == 0 && table_data.rows.len() > 1 {
                self.emit_table_line(super::table::rule(
                    &widths,
                    super::table::RulePos::Mid,
                    border,
                    border,
                ));
            } else if row_index + 1 < table_data.rows.len() {
                self.emit_table_line(super::table::rule(
                    &widths,
                    super::table::RulePos::Row,
                    border,
                    super::table::faint_rule_style(),
                ));
            }
        }
        self.emit_table_line(super::table::rule(
            &widths,
            super::table::RulePos::Bottom,
            border,
            border,
        ));
        self.separate_next_block = true;
    }

    fn render_stacked_table(&mut self, rows: Vec<Vec<Line<'static>>>) {
        let headers = rows.first().cloned().unwrap_or_default();
        for (row_index, row) in rows.iter().skip(1).enumerate() {
            if row_index > 0 {
                self.emit_blank_line();
            }
            for (column, value) in row.iter().enumerate() {
                let mut line = headers.get(column).cloned().unwrap_or_default();
                if !line.spans.is_empty() {
                    line.spans.push(Span::styled(": ", theme().muted()));
                }
                line.spans.extend(value.spans.clone());
                self.emit_wrapped_line(rich_from_line(line), None, false);
            }
        }
    }

    fn emit_table_line(&mut self, mut line: Line<'static>) {
        let mut prefix = self.container_prefix(true);
        prefix.spans.append(&mut line.spans);
        self.lines.push(prefix);
    }
}

/// A wrapped display line: its collapsed spans plus the clickable link runs
/// `(start_col, end_col, link_id)` in columns relative to the content start
/// (before any container prefix is prepended).
struct WrappedLine {
    spans: Vec<Span<'static>>,
    links: Vec<(usize, usize, usize)>,
}

#[derive(Clone, Copy)]
struct StyledCharacter {
    character: char,
    width: usize,
    style: Style,
    link: Option<usize>,
}

/// Wrap link-tagged runs to `width`, returning display lines that carry both the
/// collapsed spans and the link regions the wrap produced. `hard_wrap` breaks
/// anywhere (rules, code); otherwise it breaks at whitespace, splitting a lone
/// token only when it alone exceeds the width.
fn wrap_rich(spans: Vec<RichSpan>, width: usize, hard_wrap: bool) -> Vec<WrappedLine> {
    let mut characters = Vec::new();
    for span in spans {
        for character in span.content.chars() {
            characters.push(StyledCharacter {
                character,
                width: character.width().unwrap_or(0),
                style: span.style,
                link: span.link,
            });
        }
    }
    let char_lines = if hard_wrap {
        hard_wrap_chars(characters, width)
    } else {
        soft_wrap_chars(characters, width)
    };
    char_lines.iter().map(|line| finalize_line(line)).collect()
}

/// Word-wrap the table path, which has no link semantics; the surviving caller of
/// the plain-`Line` shape. Preserves the line-level style/alignment tables set.
fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    let style = line.style;
    let alignment = line.alignment;
    let mut characters = Vec::new();
    for span in line.spans {
        for character in span.content.chars() {
            characters.push(StyledCharacter {
                character,
                width: character.width().unwrap_or(0),
                style: span.style,
                link: None,
            });
        }
    }
    soft_wrap_chars(characters, width)
        .iter()
        .map(|chars| {
            let mut line = Line::from(finalize_line(chars).spans);
            line.style = style;
            line.alignment = alignment;
            line
        })
        .collect()
}

fn soft_wrap_chars(characters: Vec<StyledCharacter>, width: usize) -> Vec<Vec<StyledCharacter>> {
    let mut wrapped: Vec<Vec<StyledCharacter>> = vec![Vec::new()];
    let mut used = 0usize;
    let mut pending_whitespace: Option<&[StyledCharacter]> = None;
    let mut cursor = 0;

    while cursor < characters.len() {
        let whitespace = characters[cursor].character.is_whitespace();
        let end = characters[cursor..]
            .iter()
            .position(|character| character.character.is_whitespace() != whitespace)
            .map_or(characters.len(), |offset| cursor + offset);
        let token = &characters[cursor..end];

        if whitespace {
            pending_whitespace = Some(token);
            cursor = end;
            continue;
        }

        let whitespace_width = pending_whitespace.map_or(0, token_width);
        let word_width = token_width(token);
        if used > 0
            && used
                .saturating_add(whitespace_width)
                .saturating_add(word_width)
                > width
        {
            wrapped.push(Vec::new());
            used = 0;
        } else if let Some(whitespace) = pending_whitespace {
            wrapped
                .last_mut()
                .expect("wrapped output always has a line")
                .extend_from_slice(whitespace);
            used = used.saturating_add(whitespace_width);
        }
        pending_whitespace = None;

        for character in token {
            if used > 0 && used.saturating_add(character.width) > width {
                wrapped.push(Vec::new());
                used = 0;
            }
            wrapped
                .last_mut()
                .expect("wrapped output always has a line")
                .push(*character);
            used = used.saturating_add(character.width);
        }
        cursor = end;
    }

    wrapped
}

fn hard_wrap_chars(characters: Vec<StyledCharacter>, width: usize) -> Vec<Vec<StyledCharacter>> {
    let mut wrapped: Vec<Vec<StyledCharacter>> = vec![Vec::new()];
    let mut used = 0usize;
    for character in characters {
        if used > 0 && used.saturating_add(character.width) > width {
            wrapped.push(Vec::new());
            used = 0;
        }
        wrapped
            .last_mut()
            .expect("wrapped output always has a line")
            .push(character);
        used = used.saturating_add(character.width);
    }
    wrapped
}

/// Collapse a wrapped display line's characters into style-merged spans (matching
/// the previous span output) while recording each contiguous same-id link run's
/// column span.
fn finalize_line(characters: &[StyledCharacter]) -> WrappedLine {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut links = Vec::new();
    let mut column = 0usize;
    let mut run: Option<(usize, usize)> = None;
    for character in characters {
        if let Some(span) = spans.last_mut()
            && span.style == character.style
        {
            span.content.to_mut().push(character.character);
        } else {
            spans.push(Span::styled(character.character.to_string(), character.style));
        }
        match (run, character.link) {
            (Some((id, _)), Some(current)) if id == current => {}
            (Some((id, start)), _) => {
                links.push((start, column, id));
                run = character.link.map(|current| (current, column));
            }
            (None, Some(current)) => run = Some((current, column)),
            (None, None) => {}
        }
        column = column.saturating_add(character.width);
    }
    if let Some((id, start)) = run {
        links.push((start, column, id));
    }
    WrappedLine { spans, links }
}

fn token_width(token: &[StyledCharacter]) -> usize {
    token.iter().fold(0usize, |width, character| {
        width.saturating_add(character.width)
    })
}

/// Convert a plain `Line` (code, rules, stacked-table rows — none carry link
/// semantics) into the renderer's tagged runs for emission.
fn rich_from_line(line: Line<'static>) -> Vec<RichSpan> {
    line.spans
        .into_iter()
        .map(|span| RichSpan {
            content: span.content.into_owned(),
            style: span.style,
            link: None,
        })
        .collect()
}

/// Whether a link target is worth making clickable — external URLs and in-page
/// heading anchors. Relative asset paths stay styled but non-interactive.
fn is_openable_link(text: &str) -> bool {
    text.starts_with('#')
        || text.starts_with("https://")
        || text.starts_with("http://")
        || text.starts_with("mailto:")
}

/// Slugify heading text into a GitHub-style anchor: lowercased, alphanumerics /
/// `_` / `-` kept, whitespace runs collapsed to single `-`, edges trimmed.
fn heading_anchor(text: &str) -> String {
    let mut anchor = String::with_capacity(text.len());
    let mut separator = false;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() || character == '_' || character == '-' {
            if separator && !anchor.is_empty() && !anchor.ends_with('-') {
                anchor.push('-');
            }
            separator = false;
            anchor.push(character);
        } else if character.is_whitespace() {
            separator = true;
        }
    }
    anchor.trim_matches('-').to_string()
}

#[cfg(test)]
mod wrap_tests {
    use super::*;
    use ratatui::style::Modifier;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// Render just the display lines (URLs shown) for the many tests that only
    /// assert on rendered text.
    fn render_lines(source: &str, width: usize) -> Vec<Line<'static>> {
        render_text_chunk(source, width, true).lines
    }

    #[test]
    fn wraps_at_words_instead_of_the_panel_edge() {
        let wrapped = wrap_line(Line::from("alpha beta"), 7);

        assert_eq!(
            wrapped.iter().map(text).collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
    }

    #[test]
    fn punctuation_stays_with_a_word_across_style_boundaries() {
        let emphasis = Style::new().add_modifier(Modifier::ITALIC);
        let wrapped = wrap_line(
            Line::from(vec![
                Span::raw("Hello "),
                Span::styled("world", emphasis),
                Span::raw("! again."),
            ]),
            13,
        );

        assert_eq!(
            wrapped.iter().map(text).collect::<Vec<_>>(),
            ["Hello world!", "again."]
        );
        assert_eq!(wrapped[0].spans[1].style, emphasis);
        assert_eq!(wrapped[0].spans[1].content, "world");
        assert_eq!(wrapped[0].spans[2].content, "!");
    }

    #[test]
    fn rendered_markdown_does_not_orphan_punctuation_after_emphasis() {
        let lines = render_lines("Hello **world**! again.", 13);
        let visible: Vec<String> = lines
            .iter()
            .map(text)
            .filter(|line| !line.is_empty())
            .collect();

        assert_eq!(visible, ["Hello world!", "again."]);
    }

    #[test]
    fn fenced_code_uses_the_previous_open_rail_frame() {
        let lines = render_lines("```rust\nfn main() {}\n```", 40);
        let visible: Vec<String> = lines.iter().map(text).collect();

        assert_eq!(visible, ["╭─ rust", "│ fn main() {}", "╰─"]);
        assert_eq!(lines[0].spans[0].style, theme().md_blockquote());
        assert_eq!(lines[1].spans[0].style, theme().md_blockquote());
        assert_eq!(lines[2].spans[0].style, theme().md_blockquote());
    }

    #[test]
    fn every_wrapped_code_row_keeps_the_frame_rail() {
        let lines = render_lines("```\nabcdefgh\n```", 8);
        let visible: Vec<String> = lines.iter().map(text).collect();

        assert_eq!(visible, ["╭─", "│ abcdef", "│ gh", "╰─"]);
    }

    #[test]
    fn gfm_table_uses_the_shared_theme_grid() {
        let source = concat!(
            "| Hi  | sdf | sdf | s   | s   |\n",
            "|-----|-----|-----|-----|-----|\n",
            "| sdf | sfd | sdf | sdf | sdf |\n",
            "| g   | g   | ss  | h   | r   |\n",
        );
        let lines = render_lines(source, 50);
        let visible: Vec<String> = lines.iter().map(text).collect();

        assert!(visible[0].starts_with(theme().glyphs().borders.line_set().top_left));
        assert!(visible[1].contains("Hi"));
        assert!(visible[2].contains(theme().glyphs().borders.line_set().cross));
        assert!(
            visible
                .last()
                .unwrap()
                .starts_with(theme().glyphs().borders.line_set().bottom_left)
        );
        assert!(!visible.iter().any(|line| line.contains("-----")));
    }

    #[test]
    fn lists_are_flush_left_with_hanging_continuations() {
        let source = concat!(
            "1.  This is a list item with two paragraphs. Lorem ipsum dolor\n",
            "    sit amet, consectetuer adipiscing elit.\n",
            "\n",
            "    Vestibulum enim wisi, viverra nec.\n",
            "\n",
            "2.  Suspendisse id sem.\n",
        );
        let visible: Vec<String> = render_lines(source, 72).iter().map(text).collect();

        assert_eq!(
            visible[0],
            "1. This is a list item with two paragraphs. Lorem ipsum dolor"
        );
        assert_eq!(visible[1], "   sit amet, consectetuer adipiscing elit.");
        assert_eq!(visible[2], "   ");
        assert_eq!(visible[3], "   Vestibulum enim wisi, viverra nec.");
        assert_eq!(visible[4], "2. Suspendisse id sem.");

        let bullets: Vec<String> = render_lines("* Hello\n* Test", 30)
            .iter()
            .map(text)
            .collect();
        assert_eq!(bullets, ["- Hello", "- Test"]);
    }

    #[test]
    fn blockquote_in_a_list_item_follows_the_list_indent() {
        let source = "*   A list item with a blockquote:\n\n    > This is a blockquote\n    > inside a list item.";
        let visible: Vec<String> = render_lines(source, 40).iter().map(text).collect();

        assert_eq!(visible[0], "- A list item with a blockquote:");
        // The separator between the item text and the blockquote sits at the list
        // indent only — the vertical rail must not leak onto the blank line.
        assert_eq!(visible[1].trim_end(), "");
        assert!(!visible[1].contains('│'), "{visible:?}");
        assert!(
            visible.iter().any(|line| line == "  │ This is a blockquote"),
            "{visible:?}"
        );
    }

    #[test]
    fn indented_code_preserves_lines_and_receives_a_frame() {
        let source = "Here is code:\n\n    tell application \"Foo\"\n        beep\n    end tell\n";
        let visible: Vec<String> = render_lines(source, 40).iter().map(text).collect();

        assert!(visible.windows(5).any(|lines| lines
            == [
                "╭─",
                "│ tell application \"Foo\"",
                "│     beep",
                "│ end tell",
                "╰─",
            ]));
    }

    #[test]
    fn nested_quotes_use_composable_vertical_rails() {
        let visible: Vec<String> = render_lines(
            "> ## This is a header.\n>\n> 1. First\n> 2. Second\n>\n> Here's code:\n>\n>     return value\n",
            40,
        )
        .iter()
        .map(text)
        .collect();

        assert!(visible.iter().any(|line| line == "│ ## This is a header."));
        assert!(visible.iter().any(|line| line == "│ 1. First"));
        assert!(visible.iter().any(|line| line == "│ ╭─"));
        assert!(visible.iter().any(|line| line == "│ │ return value"));

        let nested: Vec<String> = render_lines("> simple quote\n>> second level quote", 40)
            .iter()
            .map(text)
            .collect();
        assert!(nested.iter().any(|line| line == "│ simple quote"));
        assert!(nested.iter().any(|line| line == "│ │ second level quote"));
    }

    #[test]
    fn thematic_breaks_and_highlights_have_terminal_styles() {
        for marker in ["***", "---", "___", "_____________________________________"] {
            let lines = render_lines(marker, 12);
            assert_eq!(lines.len(), 1, "{marker}");
            assert_eq!(lines[0].width(), 12, "{marker}");
            assert_eq!(lines[0].spans[0].style, theme().muted(), "{marker}");
        }

        let lines = render_lines("These are ==very important words==.", 40);
        let marked = lines[0]
            .spans
            .iter()
            .find(|span| span.content == "very important words")
            .unwrap();
        assert!(marked.style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn link_names_stay_blue_with_a_faint_target_trailer() {
        let chunk = render_text_chunk("See [the docs](https://example.com) now.", 60, true);
        let spans = &chunk.lines[0].spans;

        let name = spans
            .iter()
            .find(|span| span.content == "the docs")
            .unwrap();
        assert_eq!(name.style, theme().md_link());
        let trailer = spans
            .iter()
            .find(|span| span.content == " (https://example.com)")
            .unwrap();
        assert_eq!(trailer.style, theme().muted());

        // The name — not the trailer — is the recorded clickable region.
        assert_eq!(chunk.links.len(), 1);
        assert_eq!(chunk.links[0].target, "https://example.com");
        assert_eq!((chunk.links[0].start, chunk.links[0].end), (4, 12));
    }

    #[test]
    fn autolinks_drop_the_redundant_target_trailer() {
        let chunk = render_text_chunk("<https://example.com>", 60, true);
        let rendered: String = chunk.lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        assert_eq!(rendered, "https://example.com");
        // The bare URL name is itself the clickable region.
        assert_eq!(chunk.links.len(), 1);
        assert_eq!(chunk.links[0].target, "https://example.com");
    }

    #[test]
    fn hard_wraps_only_tokens_wider_than_the_panel() {
        let wrapped = wrap_line(Line::from("abcdefgh"), 4);

        assert_eq!(
            wrapped.iter().map(text).collect::<Vec<_>>(),
            ["abcd", "efgh"]
        );
    }

    #[test]
    fn wrapping_uses_terminal_cell_width() {
        let wrapped = wrap_line(Line::from("ab 界!"), 4);

        assert_eq!(wrapped.iter().map(text).collect::<Vec<_>>(), ["ab", "界!"]);
    }
}
