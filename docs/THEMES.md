# Themes

The TUI's look is defined by TOML theme files in `<config-dir>/themes/`
(`~/.config/journal/themes/` by default, next to `config.toml`). The bundled
themes are written there on first use and are never overwritten afterwards —
edits to them survive upgrades. Any `.toml` file in the directory is a theme;
the file stem is the theme name.

Bundled themes: `blossom` (default), `journal`, `classic`, `e-ink`, `fjord`,
`grove`, `matcha`, `tokyonight`, `catppuccin`, `rose-pine`. `classic` is the
terminal-default look the app has without any theme; `e-ink` is pure
black-and-white for monochrome displays.

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
(auto → dark → light), Enter applies and saves to `config.toml`, Esc reverts
theme, chrome, and mode. Broken theme files are listed but can't be applied.

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

### `[chrome]`

| Key | Form | Default |
|---|---|---|
| `default_style` | `"flat"` \| `"bordered"` | `bordered` |
| `scrim` | `0.0`–`1.0` | `0.0` |

`flat` separates surfaces by background layers; `bordered` draws boxes.
`scrim` dims the screen behind dialogs; `0.0` uses the classic DIM-modifier
fallback. Requires RGB surface colors to blend.

### `[surfaces]` — background layers, base to top

| Key | Default |
|---|---|
| `background` | terminal default |
| `panel` | ← `background` |
| `dialog` | ← `panel` |
| `element` | ← `panel` |

`background` sits under every frame, `panel` under panels and toasts, `dialog`
under dialogs and modals, `element` under inputs and interactive chips.

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

Focused titles, current-item markers, the flat focus stripe.

### `[status]`

| Key | Default |
|---|---|
| `success` / `warning` / `error` / `info` | `green` / `yellow` / `red` / `blue` |

### `[borders]`

| Key | Form | Default |
|---|---|---|
| `style` | `plain` \| `rounded` \| `double` \| `thick` \| `ascii` | `plain` |
| `normal` | token | ANSI `244` |
| `subtle` | token | ANSI `240` |
| `focused` | token | terminal ink (rendered BOLD) |
| `unfocused` | token | terminal ink |

`style` picks the box-drawing set for panels, cards, and table grids. Focused
panels also thicken their border (except `ascii`, which carries focus on
weight alone). `normal` outlines cards, `subtle` draws inter-row table rules.

### `[interaction]`

| Key | Default | Notes |
|---|---|---|
| `selection` | inverted (REVERSED) | a `bg` requires an explicit `fg` |
| `hover` | `element` lifted one step | `bg`-only is fine — it layers under the row's ink |
| `button` | ← `selection` | a `bg` requires an explicit `fg` |
| `key_hint` | inverted + bold | the footer/dialog key chips |
| `cursor` | terminal cursor | editor/input cursor while not selecting |
| `cursor_line` | none | line highlight under the editor cursor |

### `[scrollbar]`

| Key | Default |
|---|---|
| `thumb` | ← `borders.focused` |
| `track` | terminal default |

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
| `groove` | glyph | `·` (empty delta bar) |
| `bar_center` | glyph | `│` (delta/mood bar center) |
| `mood_stroke` | glyph | `─` (mood bar fill) |

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

### `[glyphs]` — identity characters

| Key | Default |
|---|---|
| `selection_marker` | follows the chrome: `●` flat, `>` bordered |
| `focus_stripe` | `┃` (focused panel edge, flat chrome) |
| `toast_edge` | `┃` (toast card accents) |
| `tab_separator` | `·` |
| `divider` | `━` (month headers, "Archived") |

## The monochrome contract

Modifiers that carry meaning are applied in code and cannot be removed by a
theme: signed values are bold, secondary ink is dim, the selection fallback is
inverted, and chart series can differ by glyph. A theme chooses colors and
identity glyphs; it can never make a positive value render as plain body text.
The `e-ink` theme is the reference: pure black-and-white, with chart series
distinguished by three distinct fill glyphs instead of hue.

## Chrome styles

- **Flat** — surfaces separate by background layer (`background` → `panel` →
  `dialog`/`element`), focused panels get a `focus_stripe` down the left edge,
  and the selection marker is `●`. Needs a theme with real surface colors.
- **Bordered** — the classic drawn borders; focus reads through thick + bold
  border strokes, and the selection marker is `>`.

Themes declare their preferred chrome in `chrome.default_style`; the `[ui] chrome`
setting (or `b` in the picker) forces one on every theme.
