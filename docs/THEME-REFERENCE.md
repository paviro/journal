# Theme reference

The file format and full token reference for writing your own themes. For the
bundled themes — where they live, and what they look like — see the
[gallery](THEMES.md).

Any `.toml` file in the themes directory is a theme, named after its file stem.
Each theme needs `schema_version = 1`; other versions or unknown keys are rejected.

## Value forms

- **Color** — `"none"` (terminal default), a name (`"cyan"`), `"#rrggbb"`, an ANSI
  index (`"0"`–`"255"`), or a `[palette]` name.
- **Variant** — any color position also accepts `{ dark = "...", light = "..." }`,
  resolved once at load by `color_mode`.
- **Palette** — `[palette]` names colors reused elsewhere. An entry may be a
  `{ dark, light }` pair, and may point at another entry by name — which can point
  at a third, and so on, resolved down the chain. A reference that loops back on
  itself is an error.
- **Token** — a bare color (foreground) or a style table
  `{ fg, bg, bold, dim, reversed, underlined }`.
- **Fill** — `{ glyph = "▓", color = "..." }`.
- **Glyph** — exactly one character.

Unknown keys anywhere are errors, so typos fail loudly.

## Sections

All sections and keys are optional; omitted keys keep the terminal-default look.
`←` marks a key that inherits from another when omitted. Each section's glyphs live
in its own `[<section>.glyphs]` table.

### `[chrome]`

| Key | Form | Default |
|---|---|---|
| `default_style` | `flat` \| `bordered` | `bordered` |
| `scrim` | `0.0`–`1.0` | `0.0` |

`flat` separates surfaces by background layer; `bordered` draws boxes.

`scrim` is how much the rest of the screen is darkened when a dialog opens, so the
dialog reads as floating above a dimmed backdrop. `0.0` leaves the background as-is;
`1.0` blends it fully to black. The smooth blend only works on true-color surface
colors — with palette or terminal-default colors (or `scrim = 0.0`) there's nothing
to blend, so the backdrop falls back to a plain DIM instead (not themeable — part
of the [monochrome contract](#monochrome-contract)).

### `[surfaces]` — layers, base to top

| Key | Default |
|---|---|
| `base` | terminal default |
| `content` | ← `base` |
| `dialog` | ← `content` |
| `element` | ← `content` |
| `footer` | ← `base` |

`base` underlies every frame and full-screen modals, `content` the main panels and
toasts, `dialog` dialogs, `element` inputs/cards/chips, `footer` the hint bar.

### `[text]`

| Key | Default |
|---|---|
| `body` | terminal ink |
| `muted` | terminal ink (DIM) |
| `heading` | ← `body` (BOLD) |
| `placeholder` | ← `muted` (DIM) |

### `[accents]`

| Key | Default |
|---|---|
| `primary` | `cyan` |
| `secondary` | ← `primary` |
| `tertiary` | ← `secondary` |

`primary` styles focused titles, current-item markers, and the flat focus stripe;
`secondary` the active tab; `tertiary` has no dedicated site. All three are seeded
as palette names, so any token can reference them by name (a theme's own
`[palette]` entry of the same name wins).

### `[status]`

`success` / `warning` / `error` / `info`, defaulting to `green` / `yellow` /
`red` / `blue`.

### `[borders]`

| Key | Form | Default |
|---|---|---|
| `style` | `plain` \| `rounded` \| `double` \| `thick` \| `ascii` | `plain` |
| `focused_style` | same names | thick promotion |
| `normal` | token | ANSI `244` |
| `subtle` | token | ANSI `240` |
| `focused` | token | terminal ink (BOLD) |
| `unfocused` | token | terminal ink |
| `divider` | token | ← `text.muted` dimmed |
| `card` | token | ← `normal` |

`style` picks the box-drawing set for panels, cards, and tables. Focused panels
thicken their border; `focused_style` replaces that promotion with a set of your
choosing. `subtle` draws inter-row table rules.

**`[borders.glyphs]` / `[borders.focused_glyphs]`** — per-character overrides
(`top_left`, `top_right`, `bottom_left`, `bottom_right`, `horizontal`,
`vertical`); omitted keys inherit the base style, and table junctions
(`├ ┤ ┬ ┴ ┼`) always do. Two standalone glyphs live here: `focus_stripe` (`┃`, the
flat-chrome focused edge, in the `focused` color) and `divider` (`━`, the month /
"Archived" rule, in the `borders.divider` color). Setting only those keeps the base `style`;
switching to a custom set needs a real box glyph. See `synthwave.toml` and
`vaporwave.toml`.

### `[interaction]`

| Key | Default | Notes |
|---|---|---|
| `selection` | REVERSED | a `bg` needs an explicit `fg` |
| `hover` | `element` +1 step | `bg`-only is fine |
| `button` | ← `selection` | a `bg` needs an explicit `fg` |
| `button_hover` | underline | button chip under the mouse |
| `key_hint` | inverted + bold | footer/dialog key chips |
| `cursor` | terminal cursor | editor/input cursor |
| `cursor_line` | none | line under the editor cursor |

The selected row is shown by `selection` alone — no marker glyph.

### `[scrollbar]`

| Key | Default |
|---|---|
| `thumb` | ← `borders.focused` |
| `track` | terminal default |
| `arrow` | ← `thumb` |

`[scrollbar.glyphs]`: `thumb` (`█`), `track` (`║`), `up` (`▲`), `down` (`▼`).

### `[charts]`

| Key | Form | Default |
|---|---|---|
| `positive` | fill | `▓` green (BOLD) |
| `neutral` | fill | `▓` (DIM) |
| `negative` | fill | `▓` red (BOLD) |
| `bar` | fill | `▓` cyan |
| `track` | fill | `░` (DIM) |
| `baseline` | `{ glyph, color }` | `┈`, ← `text.muted` dimmed |
| `label` | token | ← `text.muted` dimmed |

`[charts.glyphs]`: `groove` (`·`), `bar_center` (`│`), `mood_stroke` (`─`).

### `[markdown]`

| Key | Default |
|---|---|
| `heading` | terminal ink |
| `heading3` | ← `heading` |
| `link` | underlined |
| `code` | terminal ink |
| `blockquote` | terminal ink |

H2 is body ink + bold by design; only H1 and H3 are themeable.

**`[markdown.syntax]`** — fenced-code highlighting. Sixteen keys: `comment`,
`keyword`, `string`, `string_escape`, `number`, `constant`, `function`, `type`,
`variable`, `property`, `operator`, `punctuation`, `attribute`, `tag`, `label`,
`error`. An omitted key renders that category as plain code; omitting the table
disables highlighting.

### `[toast]` / `[tabs]`

`[toast.glyphs]`: `edge` (`┃`, toast accents on flat chrome) and `progress` (`─`,
the dismissal countdown line along the bottom edge). `[tabs]` sets
`separator` (← `text.muted` dimmed); `[tabs.glyphs]`: `separator` (`·`).

## Monochrome contract

So meaning survives without color, some distinctions are enforced in code and
can't be themed away: positive and negative chart bars are bold (neutral ones dim),
muted ink is dim, the selection fallback is inverted, and chart series are set apart
by glyph as well as hue. A theme picks the colors and glyphs, but can never flatten
a positive value into plain body text.

`eclipse` is the reference case — pure black-and-white, where those enforced
modifiers and the three chart fill glyphs carry every distinction on their own.

## Chrome styles

- **Flat** — surfaces separate by background layer (`base` → `content` →
  `dialog`/`element`), focused panels get a `focus_stripe` down the left edge, and
  the selection marker is `●`. Needs a theme with real surface colors.
- **Bordered** — drawn borders; focus reads through thick + bold strokes, and the
  selection marker is `>`.

Themes declare a preference in `chrome.default_style`; `[ui] chrome` (or `b` in
the picker) forces one everywhere.
