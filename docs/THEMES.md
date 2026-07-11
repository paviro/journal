# Themes

The TUI's look is defined by TOML theme files in `<config-dir>/themes/`
(`~/.config/journal/themes/` by default, next to `config.toml`). The bundled
themes are written there on first use and are never overwritten afterwards —
edits to them survive upgrades. Any `.toml` file in the directory is a theme;
the file stem is the theme name.

Bundled themes: `blossom` (default), `journal`, `classic`, `e-ink`, `fjord`,
`grove`, `matcha`, `tokyonight`, `catppuccin`, `rose-pine`, `dungeon`,
`synthwave`, `crt`, `cyberpunk`, `vaporwave`. `classic` is the terminal-default
look the app has without any theme; `e-ink` is pure black-and-white for
monochrome displays; the last five are bold, high-color looks that lean on the
accent, structural, and glyph tokens below.

A missing or broken configured theme falls back to the built-in `blossom` with
a warning on stderr — the app always starts.

## Config keys

```toml
[ui]
theme = "blossom"        # file stem of a theme in <config-dir>/themes/
color_mode = "auto"      # auto | dark | light — picks the { dark, light } variant
chrome = "default"       # default | flat | bordered — default follows the theme
```

- `color_mode = "auto"` uses the terminal background, queried (OSC 10/11) once
  at startup; an unknown answer counts as dark.
- `chrome` forces flat or bordered chrome on every theme; `default` uses each
  theme's own `chrome.default_style`.

## Picker and live reload

`,` opens the settings menu; `t` (or Enter) opens the theme picker. Up/Down
previews the highlighted theme live, `b` cycles the chrome override
(default → flat → bordered), `m` cycles the color mode
(auto → dark → light; hidden on themes without dark/light variants), Enter
applies and saves to `config.toml`, Esc reverts theme, chrome, and mode.
Broken theme files are listed but can't be applied.

Edits to the **active** theme's file hot-reload (debounced ~400 ms). A broken
edit shows an error toast and keeps the current theme; the next valid save
loads.

## Value forms

- **Color**: `"none"` (terminal default), a name (`"cyan"`), `"#rrggbb"`, an
  ANSI index (`"0"`–`"255"`), or a `[palette]` entry name.
- **Dark/light variant**: every color position also accepts
  `{ dark = "...", light = "..." }`; the variant is resolved once at load by
  `color_mode`.
- **Palette**: `[palette]` defines named colors reused by name elsewhere.
  Entries may themselves be `{ dark, light }` pairs but cannot reference other
  entries.
- **Token**: a bare color (used as the foreground) or a style table
  `{ fg, bg, bold, dim, reversed, underlined }`.
- **Fill**: `{ glyph = "▓", color = "..." }` — a repeated chart character plus
  its color.
- **Glyph**: exactly one character.

Unknown keys anywhere are errors, so typos fail loudly instead of silently
doing nothing.

## Sections

All sections and keys are optional; omitted keys keep the classic
terminal-default look. `←` marks a key that inherits from another when omitted.
Every glyph a section owns lives in a `[<section>.glyphs]` table of its own
(`[borders.glyphs]`, `[interaction.glyphs]`, `[scrollbar.glyphs]`,
`[charts.glyphs]`, `[toast.glyphs]`, `[tabs.glyphs]`).

### `[chrome]`

| Key | Form | Default |
|---|---|---|
| `default_style` | `"flat"` \| `"bordered"` | `bordered` |
| `scrim` | `0.0`–`1.0` | `0.0` |

`flat` separates surfaces by background layers; `bordered` draws boxes.
`scrim` blends the screen toward black behind dialogs and requires RGB surface
colors. On non-RGB (monochrome) terminals it falls back to a DIM modifier; that
fallback is applied in code and is deliberately not themeable — it is part of
the monochrome contract below.

### `[surfaces]` — surface layers, base to top

| Key | Default |
|---|---|
| `base` | terminal default |
| `content` | ← `base` |
| `dialog` | ← `content` |
| `element` | ← `content` |
| `footer` | ← `base` |

`base` sits under every frame and fills full-screen modal screens (unlock,
device access), `content` under the main panels and toasts, `dialog` under
dialogs and modals, `element` under inputs, cards, and interactive chips, and
`footer` under the hint bar — flush with `base` unless a theme tints it.

### `[text]`

| Key | Default |
|---|---|
| `body` | terminal ink |
| `muted` | terminal ink (rendered DIM) |
| `heading` | ← `body` (rendered BOLD) |
| `placeholder` | ← `muted` (rendered DIM) |

### `[accents]`

| Key | Default |
|---|---|
| `primary` | `cyan` |
| `secondary` | ← `primary` |
| `tertiary` | ← `secondary` |

`primary` styles focused titles, current-item markers, and the flat focus
stripe. `secondary` styles the active tab (so a theme can split it from the
primary hue its titles use). `tertiary` has no dedicated render site.

All three are also seeded as palette names, so any color token can ride a hero
hue by name — `fg = "secondary"`, `bg = "tertiary"` — without redeclaring it. A
theme's own `[palette]` entry of the same name wins.

### `[status]`

| Key | Default |
|---|---|
| `success` / `warning` / `error` / `info` | `green` / `yellow` / `red` / `blue` |

### `[borders]`

| Key | Form | Default |
|---|---|---|
| `style` | `plain` \| `rounded` \| `double` \| `thick` \| `ascii` | `plain` |
| `focused_style` | same names | thick promotion |
| `normal` | token | ANSI `244` |
| `subtle` | token | ANSI `240` |
| `focused` | token | terminal ink (rendered BOLD) |
| `unfocused` | token | terminal ink |
| `divider_style` | token | ← `text.muted` dimmed (the divider rule's ink) |
| `card` | token | ← `normal` (entry/stat card outlines) |

`style` picks the box-drawing set for panels, cards, and table grids. Focused
panels also thicken their border (except `ascii` and custom sets, which carry
focus on weight alone); `focused_style` replaces that promotion with a set of
your choosing. `subtle` draws inter-row table rules.

### `[borders.glyphs]` / `[borders.focused_glyphs]` — custom sets

Per-character overrides on `style` (and on the focused set): `top_left`,
`top_right`, `bottom_left`, `bottom_right`, `horizontal`, `vertical`. Omitted
keys inherit the base style's character; table junctions (`├ ┤ ┬ ┴ ┼`) always
do. The two standalone furniture glyphs live here too: `focus_stripe` (`┃`, the
flat-chrome focused edge, drawn in the `focused` color) and `divider` (`━`, the
rule of month headers and "Archived", drawn in `divider_style`). A section that
sets only those keeps the base `style` (and its thick focus-promotion); it takes
a real box glyph to switch to a custom set. See `synthwave.toml` (✦ corners on
heavy lines) and `vaporwave.toml` (dotted edges, rounded corners).

### `[interaction]`

| Key | Default | Notes |
|---|---|---|
| `selection` | inverted (REVERSED) | a `bg` requires an explicit `fg` |
| `hover` | `element` lifted one step | `bg`-only is fine — it layers under the row's ink |
| `button` | ← `selection` | a `bg` requires an explicit `fg` |
| `button_hover` | underline | patched onto a button chip under the mouse |
| `key_hint` | inverted + bold | the footer/dialog key chips |
| `cursor` | terminal cursor | editor/input cursor while not selecting |
| `cursor_line` | none | line highlight under the editor cursor |

`[interaction.glyphs]` holds `selection_marker` — the glyph before a selected
row, defaulting to the chrome (`●` flat, `>` bordered).

### `[scrollbar]`

| Key | Default |
|---|---|
| `thumb` | ← `borders.focused` |
| `track` | terminal default |
| `arrow` | ← `thumb` (the up/down caps) |

`[scrollbar.glyphs]` sets the characters: `thumb` (`█`), `track` (`║`),
`up` (`▲`), `down` (`▼`).

### `[charts]`

| Key | Form | Default |
|---|---|---|
| `positive` | fill | `▓` green (rendered BOLD) |
| `neutral` | fill | `▓` (rendered DIM) |
| `negative` | fill | `▓` red (rendered BOLD) |
| `bar` | fill | `▓` cyan |
| `track` | fill | `░` (rendered DIM) |
| `baseline` | `{ glyph, color }` | `┈`, ← `text.muted` dimmed |
| `label` | token | ← `text.muted` dimmed |

`[charts.glyphs]` sets the chart furniture characters: `groove` (`·`, empty
delta bar), `bar_center` (`│`, delta/mood bar center), `mood_stroke` (`─`, mood
bar fill). The zero baseline pairs its glyph and color, so it stays on
`[charts]` as `baseline = { glyph, color }`.

### `[markdown]`

| Key | Default |
|---|---|
| `heading` | terminal ink |
| `heading3` | ← `heading` |
| `link` | underlined |
| `code` | terminal ink |
| `blockquote` | terminal ink |

H2 is body ink + bold by the renderer's design; only H1 and H3 are themeable.

### `[markdown.syntax]` — fenced code block highlighting

Sixteen color keys: `comment`, `keyword`, `string`, `string_escape`, `number`,
`constant`, `function`, `type`, `variable`, `property`, `operator`,
`punctuation`, `attribute`, `tag`, `label`, `error`. An omitted key renders
that category as plain code; omitting the whole table disables highlighting
entirely (the classic look).

### `[toast]`

`[toast.glyphs]` holds `edge` (`┃`, the toast card accents on flat chrome).

### `[tabs]`

| Key | Default |
|---|---|
| `separator_style` | ← `text.muted` dimmed (the separator glyph's ink) |

`[tabs.glyphs]` holds `separator` (`·`, between tab labels).

## The monochrome contract

Modifiers that carry meaning are applied in code and cannot be removed by a
theme: signed values are bold, secondary ink is dim, the selection fallback is
inverted, and chart series can differ by glyph. A theme chooses colors and
identity glyphs; it can never make a positive value render as plain body text.
The `e-ink` theme is the reference: pure black-and-white, with chart series
distinguished by three distinct fill glyphs instead of hue.

## Chrome styles

- **Flat** — surfaces separate by background layer (`base` → `content` →
  `dialog`/`element`), focused panels get a `focus_stripe` down the left edge,
  and the selection marker is `●`. Needs a theme with real surface colors.
- **Bordered** — the classic drawn borders; focus reads through thick + bold
  border strokes, and the selection marker is `>`.

Themes declare their preferred chrome in `chrome.default_style`; the `[ui] chrome`
setting (or `b` in the picker) forces one on every theme.
