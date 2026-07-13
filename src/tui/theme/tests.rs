use super::*;
use schema::parse_color;
use tempfile::tempdir;

fn parse(text: &str, mode: Mode) -> anyhow::Result<Theme> {
    // Bundled themes carry their own `schema_version` (below the file's lead
    // comment); bare inline snippets don't, so prepend one for them.
    if text
        .lines()
        .any(|line| line.starts_with("schema_version = "))
    {
        super::parse(text, mode)
    } else {
        super::parse(&format!("schema_version = 1\n{text}"), mode)
    }
}

fn bundled(name: &str) -> &'static str {
    BUNDLED
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| *text)
        .expect("bundled theme exists")
}

#[test]
fn every_bundled_theme_parses_in_both_modes() {
    for (name, text) in BUNDLED {
        for mode in [Mode::Dark, Mode::Light] {
            parse(text, mode).unwrap_or_else(|err| {
                panic!("bundled theme '{name}' failed to resolve ({mode:?}): {err:#}")
            });
        }
    }
}

#[test]
fn chrome_override_wins_over_the_theme_default() {
    set_test_theme(test_flat_theme());
    assert_eq!(theme().chrome(), ChromeStyle::Flat);
    set_chrome_override(Some(ChromeStyle::Bordered));
    assert_eq!(theme().chrome(), ChromeStyle::Bordered, "override ignored");
    set_chrome_override(None);
    assert_eq!(
        theme().chrome(),
        ChromeStyle::Flat,
        "auto must follow the theme"
    );
}

#[test]
fn default_hover_lifts_the_element_surface() {
    // Theme files written before the hover token existed (materialized
    // copies are never overwritten) must still get a visible hover: the
    // default nudges element toward white (dark) / black (light).
    let text = "[surfaces]\nbase = \"#101010\"\ncontent = \"#181818\"\nelement = \"#202020\"";
    let dark = parse(text, Mode::Dark).unwrap();
    assert_eq!(dark.hover().bg, Some(Color::Rgb(0x36, 0x36, 0x36)));
    let light = parse(
        "[surfaces]\nbase = \"#f0f0f0\"\ncontent = \"#e8e8e8\"\nelement = \"#e0e0e0\"",
        Mode::Light,
    )
    .unwrap();
    assert_eq!(light.hover().bg, Some(Color::Rgb(0xc9, 0xc9, 0xc9)));
}

#[test]
fn new_tokens_chain_to_their_parents() {
    // Each new token inherits its parent when omitted, so themes written
    // before the tokens existed keep rendering as they did.
    let theme = parse(
        "[text]\n\
         body = \"#aabbcc\"\n\
         muted = \"#334455\"\n\
         [interaction]\n\
         selection = { fg = \"#000000\", bg = \"#ffffff\" }\n\
         [borders]\n\
         focused = \"#606060\"\n\
         [markdown]\n\
         heading = \"#56b6b0\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(
        theme.heading(),
        Style::default()
            .fg(Color::Rgb(0xaa, 0xbb, 0xcc))
            .add_modifier(Modifier::BOLD)
    );
    assert_eq!(
        theme.placeholder(),
        Style::default()
            .fg(Color::Rgb(0x33, 0x44, 0x55))
            .add_modifier(Modifier::DIM)
    );
    assert_eq!(theme.button(), theme.selection());
    assert_eq!(
        theme.scrollbar_thumb(true),
        Style::default().fg(Color::Rgb(0x60, 0x60, 0x60))
    );
    assert_eq!(theme.scrollbar_track(true), Style::default());
    assert_eq!(theme.md_heading3(), theme.md_heading());
    // The editor tokens default to "no styling".
    assert_eq!(theme.cursor(), Style::default());
    assert_eq!(theme.cursor_line(), Style::default());
    // Accents: secondary chains to primary (here still the cyan default).
    assert_eq!(theme.secondary(), theme.primary());
    // Structural furniture keeps the ink it has always used.
    assert_eq!(theme.divider(), theme.muted());
    assert_eq!(theme.tab_separator(), theme.muted());
    assert_eq!(
        theme.card_border(),
        Style::default().fg(Color::Indexed(244))
    );
    // Interaction polish defaults preserve the old hardcoded behavior.
    assert_eq!(
        theme.button_hover(),
        Style::default().add_modifier(Modifier::UNDERLINED)
    );
    assert_eq!(theme.scrollbar_arrow(true), theme.scrollbar_thumb(true));
    // Unfocused, the scrollbar recedes with DIM the way the border quiets —
    // visible even though the default parts carry no colour of their own.
    assert_eq!(
        theme.scrollbar_thumb(false),
        theme
            .scrollbar_thumb(true)
            .remove_modifier(Modifier::BOLD)
            .add_modifier(Modifier::DIM)
    );
    assert_eq!(
        theme.scrollbar_track(false),
        theme
            .scrollbar_track(true)
            .remove_modifier(Modifier::BOLD)
            .add_modifier(Modifier::DIM)
    );
    assert_eq!(
        theme.scrollbar_arrow(false),
        theme
            .scrollbar_arrow(true)
            .remove_modifier(Modifier::BOLD)
            .add_modifier(Modifier::DIM)
    );
}

#[test]
fn new_tokens_resolve_explicit_values() {
    let theme = parse(
        "[text]\n\
         heading = \"#112233\"\n\
         placeholder = \"#445566\"\n\
         [accents]\n\
         secondary = \"#2de2e6\"\n\
         [interaction]\n\
         button = { fg = \"#000000\", bg = \"#aabbcc\" }\n\
         button_hover = { bold = true }\n\
         cursor = { reversed = true }\n\
         cursor_line = { bg = \"#181818\" }\n\
         [borders]\n\
         divider = \"#101010\"\n\
         card = \"#202020\"\n\
         [tabs]\n\
         separator = \"#303030\"\n\
         [scrollbar]\n\
         thumb = \"#778899\"\n\
         track = \"#223344\"\n\
         arrow = \"#404040\"\n\
         [markdown]\n\
         heading3 = \"#556677\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.heading().fg, Some(Color::Rgb(0x11, 0x22, 0x33)));
    assert_eq!(theme.placeholder().fg, Some(Color::Rgb(0x44, 0x55, 0x66)));
    assert_eq!(theme.secondary().fg, Some(Color::Rgb(0x2d, 0xe2, 0xe6)));
    assert_eq!(theme.button().bg, Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
    assert!(theme.button_hover().add_modifier.contains(Modifier::BOLD));
    assert!(theme.cursor().add_modifier.contains(Modifier::REVERSED));
    assert_eq!(theme.cursor_line().bg, Some(Color::Rgb(0x18, 0x18, 0x18)));
    assert_eq!(theme.divider().fg, Some(Color::Rgb(0x10, 0x10, 0x10)));
    assert_eq!(theme.card_border().fg, Some(Color::Rgb(0x20, 0x20, 0x20)));
    assert_eq!(theme.tab_separator().fg, Some(Color::Rgb(0x30, 0x30, 0x30)));
    assert_eq!(
        theme.scrollbar_thumb(true).fg,
        Some(Color::Rgb(0x77, 0x88, 0x99))
    );
    assert_eq!(
        theme.scrollbar_track(true).fg,
        Some(Color::Rgb(0x22, 0x33, 0x44))
    );
    assert_eq!(
        theme.scrollbar_arrow(true).fg,
        Some(Color::Rgb(0x40, 0x40, 0x40))
    );
    assert_eq!(theme.md_heading3().fg, Some(Color::Rgb(0x55, 0x66, 0x77)));
}

#[test]
fn accents_are_referenceable_by_name_in_any_token() {
    // The three accents are seeded as palette names, so any color token can
    // ride a hero hue without redeclaring it. tertiary has no dedicated render
    // site but is still nameable.
    let theme = parse(
        "[accents]\n\
         primary = \"#ff2f92\"\n\
         secondary = \"#2de2e6\"\n\
         tertiary = \"#a06bff\"\n\
         [borders]\n\
         divider = \"secondary\"\n\
         [tabs]\n\
         separator = \"tertiary\"\n\
         [text]\n\
         heading = \"primary\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.divider().fg, Some(Color::Rgb(0x2d, 0xe2, 0xe6)));
    assert_eq!(theme.tab_separator().fg, Some(Color::Rgb(0xa0, 0x6b, 0xff)));
    assert_eq!(theme.heading().fg, Some(Color::Rgb(0xff, 0x2f, 0x92)));

    // A theme's own [palette] entry of the same name wins over the seed.
    let overridden = parse(
        "[palette]\n\
         secondary = \"#010203\"\n\
         [accents]\n\
         secondary = \"#2de2e6\"\n\
         [borders]\n\
         divider = \"secondary\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(overridden.divider().fg, Some(Color::Rgb(0x01, 0x02, 0x03)));
}

#[test]
fn palette_references_resolve_transitively() {
    // An accent that names a [palette] entry, referenced in turn by another
    // token, resolves all the way through — a named hue can be reused without
    // every reference having to spell out the final color.
    let theme = parse(
        "[palette]\n\
         hero = \"#ff2d95\"\n\
         [accents]\n\
         tertiary = \"hero\"\n\
         [borders]\n\
         divider = \"tertiary\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.divider().fg, Some(Color::Rgb(0xff, 0x2d, 0x95)));

    // A reference cycle can't loop forever; it falls through to a parse error.
    assert!(
        parse(
            "[palette]\n\
             a = \"b\"\n\
             b = \"a\"\n\
             [borders]\n\
             divider = \"a\"",
            Mode::Dark,
        )
        .is_err()
    );
}

#[test]
fn button_rejects_bg_without_fg() {
    let err = parse("[interaction]\nbutton = { bg = \"#aabbcc\" }", Mode::Dark).unwrap_err();
    assert!(err.to_string().contains("interaction.button"), "{err:#}");
}

#[test]
fn glyphs_resolve_and_default() {
    let theme = parse(
        "[borders]\nstyle = \"rounded\"\n[borders.glyphs]\nfocus_stripe = \"█\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.glyphs().focus_stripe, '█');
    // A furniture-only [borders.glyphs] keeps the named base style (and its
    // thick focus-promotion), not a collapsed custom set.
    assert_eq!(theme.glyphs().borders, BorderGlyphs::Rounded);
    // Defaults untouched by a partial section.
    assert_eq!(theme.glyphs().toast_edge, '┃');
    assert_eq!(theme.glyphs().divider, '━');

    let default = Theme::terminal_default();
    assert_eq!(default.glyphs().borders, BorderGlyphs::Plain);
}

#[test]
fn glyph_tokens_must_be_one_character() {
    let err = parse("[borders.glyphs]\nfocus_stripe = \"ab\"", Mode::Dark).unwrap_err();
    assert!(
        err.to_string().contains("borders.glyphs.focus_stripe"),
        "{err:#}"
    );
}

#[test]
fn chart_baseline_merges_glyph_and_color() {
    let theme = parse(
        "[charts]\nbaseline = { glyph = \"╌\", color = \"#123456\" }",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.glyphs().chart_baseline, '╌');
    assert_eq!(
        theme.chart_baseline(),
        Style::default().fg(Color::Rgb(0x12, 0x34, 0x56))
    );
    // Each half keeps its default when the other is set alone.
    let glyph_only = parse("[charts]\nbaseline = { glyph = \"╌\" }", Mode::Dark).unwrap();
    assert_eq!(glyph_only.glyphs().chart_baseline, '╌');
    assert_eq!(
        glyph_only.chart_baseline(),
        Style::default().add_modifier(Modifier::DIM)
    );
    let color_only = parse("[charts]\nbaseline = { color = \"#123456\" }", Mode::Dark).unwrap();
    assert_eq!(color_only.glyphs().chart_baseline, '┈');
    assert_eq!(
        color_only.chart_baseline().fg,
        Some(Color::Rgb(0x12, 0x34, 0x56))
    );
}

#[test]
fn chart_glyphs_live_in_the_charts_section() {
    let theme = parse(
        "[charts.glyphs]\ngroove = \"‥\"\nbar_center = \"┋\"\nmood_stroke = \"═\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.glyphs().chart_groove, '‥');
    assert_eq!(theme.glyphs().bar_center, '┋');
    assert_eq!(theme.glyphs().mood_fill, '═');
    // Defaults untouched by a partial section.
    let bare = parse("", Mode::Dark).unwrap();
    assert_eq!(bare.glyphs().chart_groove, '·');
    assert_eq!(bare.glyphs().bar_center, '│');
    assert_eq!(bare.glyphs().mood_fill, '─');
}

#[test]
fn border_glyph_sets_cover_focus_and_ascii() {
    // A themed set thickens for focus; ascii has no thick variant, so a
    // focused ascii panel keeps its own corners (focus rides on the style).
    assert_eq!(BorderGlyphs::Rounded.block_set(false).top_left, "╭");
    assert_eq!(BorderGlyphs::Rounded.block_set(true).top_left, "┏");
    assert_eq!(BorderGlyphs::Ascii.block_set(false).top_left, "+");
    assert_eq!(BorderGlyphs::Ascii.block_set(true).top_left, "+");
    assert_eq!(BorderGlyphs::Ascii.line_set().cross, "+");
}

#[test]
fn scrollbar_glyphs_resolve_and_keep_their_defaults() {
    let theme = parse(
        "[scrollbar.glyphs]\nthumb = \"#\"\ntrack = \"|\"\nup = \"^\"\ndown = \"v\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.glyphs().scrollbar_thumb, '#');
    assert_eq!(theme.glyphs().scrollbar_track, '|');
    assert_eq!(theme.glyphs().scrollbar_up, '^');
    assert_eq!(theme.glyphs().scrollbar_down, 'v');
    // The defaults are ratatui's own vertical set, so bare themes don't change.
    let bare = parse("", Mode::Dark).unwrap();
    assert_eq!(bare.glyphs().scrollbar_thumb, '█');
    assert_eq!(bare.glyphs().scrollbar_track, '║');
    assert_eq!(bare.glyphs().scrollbar_up, '▲');
    assert_eq!(bare.glyphs().scrollbar_down, '▼');
}

#[test]
fn custom_border_glyphs_overlay_the_base_style() {
    let theme = parse(
        "[borders]\nstyle = \"rounded\"\n[borders.glyphs]\ntop_left = \"✦\"\nhorizontal = \"┄\"",
        Mode::Dark,
    )
    .unwrap();
    let set = theme.glyphs().borders.border_set();
    assert_eq!(set.top_left, "✦");
    assert_eq!(set.horizontal_top, "┄");
    assert_eq!(set.horizontal_bottom, "┄");
    // Omitted keys inherit the base style's glyph; junctions always do.
    assert_eq!(set.top_right, "╮");
    assert_eq!(theme.glyphs().borders.line_set().cross, "┼");
    // A custom set has no thick variant, so focus keeps it (like ascii).
    assert_eq!(theme.glyphs().block_set(true).top_left, "✦");
    assert!(parse("[borders.glyphs]\nhorizontal = \"--\"", Mode::Dark).is_err());
}

#[test]
fn focused_border_style_replaces_the_thick_promotion() {
    let theme = parse("[borders]\nfocused_style = \"double\"", Mode::Dark).unwrap();
    assert_eq!(theme.glyphs().block_set(false).top_left, "┌");
    assert_eq!(theme.glyphs().block_set(true).top_left, "╔");
    // Without the token the classic thick promotion stands.
    let bare = parse("", Mode::Dark).unwrap();
    assert_eq!(bare.glyphs().block_set(true).top_left, "┏");
}

#[test]
fn syntax_colors_resolve_and_default_to_reset() {
    let theme = parse(
        "[markdown.syntax]\nkeyword = \"#fab283\"\nstring = \"green\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.syntax().keyword, Color::Rgb(0xfa, 0xb2, 0x83));
    assert_eq!(theme.syntax().string, Color::Green);
    // Unset categories stay plain, so classic code blocks don't change.
    assert_eq!(theme.syntax().comment, Color::Reset);
    assert_eq!(Theme::terminal_default().syntax().keyword, Color::Reset);
}

#[test]
fn border_inactive_resolves_and_defaults_to_terminal_ink() {
    let themed = parse("[borders]\nunfocused = \"#3c3c3c\"", Mode::Dark).unwrap();
    assert_eq!(
        themed.inactive_border(),
        Style::default().fg(Color::Rgb(0x3c, 0x3c, 0x3c))
    );
    // Theme files from before the token existed keep the classic look.
    let bare = parse("", Mode::Dark).unwrap();
    assert_eq!(bare.inactive_border(), Style::default());
}

#[test]
fn dialog_defaults_to_content_for_existing_theme_files() {
    let theme = parse(
        "[surfaces]\nbase = \"#101010\"\ncontent = \"#181818\"",
        Mode::Dark,
    )
    .unwrap();
    assert_eq!(theme.dialog, theme.content);
}

#[test]
fn flat_bundled_themes_split_dialogs_from_content() {
    for (name, text) in BUNDLED {
        for mode in [Mode::Dark, Mode::Light] {
            let theme = parse(text, mode).unwrap();
            if theme.chrome == ChromeStyle::Flat {
                assert_ne!(
                    theme.dialog, theme.content,
                    "'{name}' dialog matches content ({mode:?})"
                );
                assert_ne!(
                    theme.dialog, theme.element,
                    "'{name}' dialog matches element ({mode:?})"
                );
            }
        }
    }
}

#[test]
fn every_bundled_theme_clears_the_contrast_floor() {
    // A cheap "renders acceptably in both modes" floor: selection must
    // never smear same-on-same, and body text must never dissolve into
    // the background.
    for (name, text) in BUNDLED {
        for mode in [Mode::Dark, Mode::Light] {
            let theme = parse(text, mode).unwrap();
            match (theme.selection.fg, theme.selection.bg) {
                (Some(fg), Some(bg)) => {
                    assert_ne!(fg, bg, "'{name}' selection is same-on-same ({mode:?})")
                }
                // Without pinned colors the inversion must carry contrast.
                _ => assert!(
                    theme.selection.add_modifier.contains(Modifier::REVERSED),
                    "'{name}' selection has neither contrast colors nor inversion ({mode:?})"
                ),
            }
            if let Some(fg) = theme.text.fg {
                assert_ne!(fg, theme.base, "'{name}' text matches its base ({mode:?})");
            }
        }
    }
}

#[test]
fn classic_is_the_builtin_fallback() {
    // classic.toml is the living spec for `terminal_default()`: the two
    // must never drift, in either mode (classic has no variant colors).
    for mode in [Mode::Dark, Mode::Light] {
        assert_eq!(
            parse(bundled("classic"), mode).unwrap(),
            Theme::terminal_default(),
            "classic.toml drifted from Theme::terminal_default ({mode:?})"
        );
    }
}

#[test]
fn terminal_default_matches_the_original_styles() {
    // The pre-theme-engine styles, pinned so the whole render test suite
    // (which never installs a theme) keeps exercising the original look.
    let theme = Theme::terminal_default();
    assert_eq!(
        theme.heading(),
        Style::default().add_modifier(Modifier::BOLD)
    );
    assert_eq!(theme.muted(), Style::default().add_modifier(Modifier::DIM));
    assert_eq!(theme.primary(), Style::default().fg(Color::Cyan));
    assert_eq!(theme.chart_bar().style, Style::default().fg(Color::Cyan));
    assert_eq!(
        theme.positive(),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    );
    assert_eq!(
        theme.negative(),
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    );
    assert_eq!(theme.neutral(), Style::default());
    assert_eq!(
        theme.active_tab(true),
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    );
    assert_eq!(
        theme.active_tab(false),
        Style::default().add_modifier(Modifier::BOLD)
    );
    assert_eq!(
        theme.inactive_tab(),
        Style::default().add_modifier(Modifier::DIM)
    );
    assert_eq!(
        theme.focus_border(),
        Style::default().add_modifier(Modifier::BOLD)
    );
    // Unfocused panels and dialog frames keep the terminal-default ink the
    // app always drew them with.
    assert_eq!(theme.inactive_border(), Style::default());
    assert_eq!(theme.dialog_border(), Style::default());
    assert_eq!(theme.faint_rule(), Style::default().fg(Color::Indexed(240)));
    assert_eq!(
        theme.card_border(),
        Style::default().fg(Color::Indexed(244))
    );
    assert_eq!(
        theme.selection(),
        Style::default().add_modifier(Modifier::REVERSED)
    );
    assert_eq!(
        theme.key_hint(),
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    );
    // Hover inherits the element surface, which is the terminal default
    // here — an invisible hover, keeping the classic look inert.
    assert_eq!(theme.hover(), Style::default().bg(Color::Reset));
    // The pass-2 tokens: buttons ride selection, headings ride text,
    // placeholders ride muted, the editor and scrollbars stay unstyled —
    // exactly the pre-token rendering.
    assert_eq!(theme.button(), theme.selection());
    assert_eq!(
        theme.heading(),
        Style::default().add_modifier(Modifier::BOLD)
    );
    assert_eq!(
        theme.placeholder(),
        Style::default().add_modifier(Modifier::DIM)
    );
    assert_eq!(theme.cursor(), Style::default());
    assert_eq!(theme.cursor_line(), Style::default());
    assert_eq!(theme.scrollbar_thumb(true), Style::default());
    assert_eq!(theme.scrollbar_track(true), Style::default());
    assert_eq!(theme.chrome(), ChromeStyle::Bordered);
    assert_eq!(theme.scrim_strength(), 0.0);
}

#[test]
fn signed_distinguishes_positive_from_negative() {
    let theme = theme();
    assert_ne!(theme.signed(1.0), theme.signed(-1.0));
}

#[test]
fn eclipse_is_monochrome_high_contrast_in_both_modes() {
    let ink_or_paper =
        |color: Color| color == Color::Rgb(0, 0, 0) || color == Color::Rgb(255, 255, 255);
    for mode in [Mode::Dark, Mode::Light] {
        let theme = parse(bundled("eclipse"), mode).unwrap();
        for (name, style) in theme.all_styles() {
            for color in [style.fg, style.bg].into_iter().flatten() {
                assert!(
                    ink_or_paper(color),
                    "eclipse `{name}` uses non-monochrome {color:?} ({mode:?})"
                );
            }
        }
        for color in [theme.base, theme.content, theme.element, theme.footer] {
            assert!(ink_or_paper(color), "eclipse surface {color:?} ({mode:?})");
        }
        assert_ne!(
            theme.dialog, theme.base,
            "eclipse dialog should lift off the base surface ({mode:?})"
        );
        assert_eq!(
            theme.dialog,
            match mode {
                Mode::Dark => Color::Rgb(0x1a, 0x1a, 0x1a),
                Mode::Light => Color::Rgb(0xf2, 0xf2, 0xf2),
            }
        );
        // Series identity must survive without hue: three distinct glyphs.
        let glyphs = [
            theme.chart_positive.glyph,
            theme.chart_neutral.glyph,
            theme.chart_negative.glyph,
        ];
        assert_eq!(
            glyphs.len(),
            glyphs
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "eclipse chart series share a glyph"
        );
        // Selection must be a true inversion, not a same-on-same smear.
        assert_ne!(theme.selection.fg, theme.selection.bg);
        // Signed values keep their weight with color stripped.
        assert!(theme.positive().add_modifier.contains(Modifier::BOLD));
        assert!(theme.negative().add_modifier.contains(Modifier::BOLD));
    }
}

#[test]
fn journal_resolves_variants_by_mode() {
    let dark = parse(bundled("journal"), Mode::Dark).unwrap();
    let light = parse(bundled("journal"), Mode::Light).unwrap();
    assert_eq!(dark.base, Color::Rgb(0x0a, 0x0a, 0x0a));
    assert_eq!(light.base, Color::Rgb(0xfc, 0xfc, 0xfc));
    assert_eq!(dark.primary.fg, Some(Color::Rgb(0x56, 0xb6, 0xb0)));
    assert_eq!(light.primary.fg, Some(Color::Rgb(0x15, 0x7d, 0x76)));
    assert_eq!(dark.chrome, ChromeStyle::Flat);
    assert!(dark.scrim > 0.0);
}

#[test]
fn palette_entries_resolve_including_variants() {
    let theme = parse(
        r##"
        [palette]
        splash = { dark = "#102030", light = "#e0e0e0" }
        flat = "#445566"

        [accents]
        primary = "splash"

        [status]
        info = "flat"
        "##,
        Mode::Light,
    )
    .unwrap();
    assert_eq!(theme.primary.fg, Some(Color::Rgb(0xe0, 0xe0, 0xe0)));
    assert_eq!(theme.info.fg, Some(Color::Rgb(0x44, 0x55, 0x66)));
}

#[test]
fn color_forms_parse() {
    assert_eq!(parse_color("none").unwrap(), Color::Reset);
    assert_eq!(parse_color("cyan").unwrap(), Color::Cyan);
    assert_eq!(
        parse_color("#336699").unwrap(),
        Color::Rgb(0x33, 0x66, 0x99)
    );
    assert_eq!(parse_color("244").unwrap(), Color::Indexed(244));
    assert!(parse_color("chartreuse-ish").is_err());
}

#[test]
fn selection_bg_without_fg_is_rejected() {
    let err = parse(
        "[interaction]\nselection = { bg = \"#ff0000\" }\n",
        Mode::Dark,
    )
    .unwrap_err();
    assert!(err.to_string().contains("selection"), "{err:#}");
}

#[test]
fn multi_char_glyphs_are_rejected() {
    let err = parse(
        "[charts]\nbar = { glyph = \"▓▓\", color = \"cyan\" }\n",
        Mode::Dark,
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("exactly one character"),
        "{err:#}"
    );
}

#[test]
fn unknown_keys_are_rejected() {
    assert!(parse("[accents]\nprimry = \"cyan\"\n", Mode::Dark).is_err());
    // The pre-restructure grab-bag section must error, not silently no-op.
    assert!(parse("[colors]\nprimary = \"cyan\"\n", Mode::Dark).is_err());
}

#[test]
fn ensure_bundled_writes_missing_but_never_overwrites() {
    let dir = tempdir().unwrap();
    let themes = dir.path().join("themes");

    ensure_bundled(&themes).unwrap();
    for (name, text) in BUNDLED {
        let on_disk = fs::read_to_string(themes.join(format!("{name}.toml"))).unwrap();
        assert_eq!(on_disk, text);
    }

    // A user-edited file survives the next materialization untouched.
    let edited = themes.join("journal.toml");
    fs::write(&edited, "[chrome]\ndefault_style = \"bordered\"\n").unwrap();
    ensure_bundled(&themes).unwrap();
    assert_eq!(
        fs::read_to_string(&edited).unwrap(),
        "[chrome]\ndefault_style = \"bordered\"\n"
    );
}

#[test]
fn load_falls_back_to_builtin_on_a_broken_theme() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let themes = themes_dir(&config_path);
    fs::create_dir_all(&themes).unwrap();
    fs::write(themes.join("broken.toml"), "surfaces = 12\n").unwrap();

    let theme = load(&config_path, "broken", Mode::Dark);
    assert_eq!(theme, builtin(DEFAULT_THEME, Mode::Dark).unwrap());

    let missing = load(&config_path, "does-not-exist", Mode::Dark);
    assert_eq!(missing, builtin(DEFAULT_THEME, Mode::Dark).unwrap());
}

#[test]
fn test_theme_override_pins_this_thread() {
    let journal = builtin("journal", Mode::Dark).unwrap();
    set_test_theme(journal);
    assert_eq!(theme(), journal);
}
