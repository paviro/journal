use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

use crate::tui::theme::{Syntax, theme};

const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "comment.documentation",
    "conditional",
    "constant",
    "constant.builtin",
    "constructor",
    "exception",
    "function",
    "function.builtin",
    "include",
    "keyword",
    "keyword.function",
    "label",
    "namespace",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "repeat",
    "string",
    "string.escape",
    "string.regex",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.member",
    "variable.parameter",
    "error",
];

struct Language {
    language: tree_sitter::Language,
    highlights: &'static str,
}

macro_rules! language {
    ($grammar:ident, $query:ident) => {
        Language {
            language: $grammar::LANGUAGE.into(),
            highlights: $grammar::$query,
        }
    };
}

fn language_for(name: &str) -> Option<Language> {
    match name.trim().to_ascii_lowercase().as_str() {
        "bash" | "sh" | "shell" | "zsh" => Some(language!(tree_sitter_bash, HIGHLIGHT_QUERY)),
        "css" | "scss" | "less" => Some(language!(tree_sitter_css, HIGHLIGHTS_QUERY)),
        "diff" | "patch" => Some(language!(tree_sitter_diff, HIGHLIGHTS_QUERY)),
        "html" | "htm" => Some(language!(tree_sitter_html, HIGHLIGHTS_QUERY)),
        "javascript" | "js" => Some(language!(tree_sitter_javascript, HIGHLIGHT_QUERY)),
        "json" => Some(language!(tree_sitter_json, HIGHLIGHTS_QUERY)),
        "python" | "py" => Some(language!(tree_sitter_python, HIGHLIGHTS_QUERY)),
        "rust" | "rs" => Some(language!(tree_sitter_rust, HIGHLIGHTS_QUERY)),
        "sql" => Some(language!(tree_sitter_sequel, HIGHLIGHTS_QUERY)),
        "swift" => Some(language!(tree_sitter_swift, HIGHLIGHTS_QUERY)),
        "toml" => Some(language!(tree_sitter_toml_ng, HIGHLIGHTS_QUERY)),
        "typescript" | "ts" => Some(Language {
            language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            highlights: tree_sitter_typescript::HIGHLIGHTS_QUERY,
        }),
        "tsx" => Some(Language {
            language: tree_sitter_typescript::LANGUAGE_TSX.into(),
            highlights: tree_sitter_typescript::HIGHLIGHTS_QUERY,
        }),
        "yaml" | "yml" => Some(language!(tree_sitter_yaml, HIGHLIGHTS_QUERY)),
        _ => None,
    }
}

thread_local! {
    /// Per-language highlight configs, cached because `HighlightConfiguration::new`
    /// recompiles the grammar's query — needless work on every code block, every
    /// frame. Keyed by the normalized fence language; the config depends only on
    /// the grammar, not the theme, so it's safe to reuse across theme changes.
    static CONFIGS: RefCell<HashMap<String, HighlightConfiguration>> =
        RefCell::new(HashMap::new());
}

pub(crate) fn highlight(language: &str, code: &str) -> Option<Vec<Line<'static>>> {
    if !theme().syntax().any_color() {
        return None;
    }
    let key = language.trim().to_ascii_lowercase();
    CONFIGS.with(|configs| {
        let mut configs = configs.borrow_mut();
        if !configs.contains_key(&key) {
            let language = language_for(&key)?;
            let mut configuration = HighlightConfiguration::new(
                language.language,
                "notema",
                language.highlights,
                "",
                "",
            )
            .ok()?;
            configuration.configure(HIGHLIGHT_NAMES);
            configs.insert(key.clone(), configuration);
        }
        let configuration = configs.get(&key)?;
        let mut highlighter = Highlighter::new();
        let events = highlighter
            .highlight(configuration, code.as_bytes(), None, |_| None)
            .ok()?;

        let syntax = theme().syntax();
        let mut active = Vec::new();
        let mut lines = vec![Line::default()];
        for event in events {
            match event.ok()? {
                HighlightEvent::Source { start, end } => {
                    let style = active
                        .last()
                        .map(|index| style_for(*index, syntax))
                        .unwrap_or_else(|| theme().md_code());
                    // Tree-sitter byte offsets are char-aligned in practice, but a
                    // non-boundary slice would panic mid-draw and take down the TUI;
                    // fall back to unhighlighted rendering instead.
                    push_source(&mut lines, code.get(start..end)?, style);
                }
                HighlightEvent::HighlightStart(Highlight(index)) => active.push(index),
                HighlightEvent::HighlightEnd => {
                    active.pop();
                }
            }
        }
        Some(lines)
    })
}

fn push_source(lines: &mut Vec<Line<'static>>, source: &str, style: Style) {
    for (index, part) in source.split('\n').enumerate() {
        if index > 0 {
            lines.push(Line::default());
        }
        if !part.is_empty() {
            lines
                .last_mut()
                .expect("syntax output always has a line")
                .spans
                .push(Span::styled(part.to_string(), style));
        }
    }
}

fn style_for(index: usize, syntax: Syntax) -> Style {
    let name = HIGHLIGHT_NAMES.get(index).copied().unwrap_or_default();
    match name {
        "comment" | "comment.documentation" => Style::new()
            .fg(syntax.comment)
            .add_modifier(Modifier::ITALIC),
        "constant" | "constant.builtin" | "boolean" => Style::new().fg(syntax.constant),
        "string" | "string.special" => Style::new().fg(syntax.string),
        "string.escape" | "string.regex" => Style::new().fg(syntax.string_escape),
        "keyword" | "keyword.function" | "conditional" | "repeat" | "exception" | "include" => {
            Style::new().fg(syntax.keyword).add_modifier(Modifier::BOLD)
        }
        "number" => Style::new().fg(syntax.number),
        "function" | "function.builtin" => Style::new().fg(syntax.function),
        "type" | "type.builtin" | "namespace" | "constructor" => Style::new().fg(syntax.r#type),
        "variable" | "variable.builtin" | "variable.parameter" | "variable.member" => {
            Style::new().fg(syntax.variable)
        }
        "property" => Style::new().fg(syntax.property),
        "operator" => Style::new().fg(syntax.operator),
        "punctuation" | "punctuation.bracket" | "punctuation.delimiter" | "punctuation.special" => {
            Style::new().fg(syntax.punctuation)
        }
        "attribute" => Style::new().fg(syntax.attribute),
        "tag" => Style::new().fg(syntax.tag),
        "label" => Style::new().fg(syntax.label),
        "error" => Style::new().fg(syntax.error),
        _ => theme().md_code(),
    }
}
