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
| `raised` | ← `content` |
| `footer` | ← `base` |

`base` underlies every frame and full-screen modals, `content` the main panels and
toasts, `dialog` dialogs, `raised` inputs/cards/chips, `footer` the hint bar.

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
| `subtle` | token | ANSI `240` |
| `focused` | token | terminal ink (BOLD) |
| `unfocused` | token | terminal ink |
| `divider` | token | ← `text.muted` dimmed |
| `card` | token | ANSI `244` |

`style` picks the box-drawing set for panels, cards, and tables. Focused panels
thicken their border; `focused_style` replaces that promotion with a set of your
choosing. Panel borders are `focused` / `unfocused`; `subtle` draws inter-row table
rules, and `card` the quieter outline of entry/journal/stat cards.

**`[borders.glyphs]` / `[borders.focused_glyphs]`** — per-character overrides
(`top_left`, `top_right`, `bottom_left`, `bottom_right`, `horizontal`,
`vertical`); omitted keys inherit the base style, and table junctions
(`├ ┤ ┬ ┴ ┼`) always do. Three standalone glyphs live here: `focus_stripe` (`┃`, the
flat-chrome focused edge, in the `focused` color), `divider` (`━`, the month /
"Archived" rule, in the `borders.divider` color), and `separator` (`─`, the plain
rule between dialog sections). Setting only those keeps the base `style`;
switching to a custom set needs a real box glyph. See `synthwave.toml` and
`vaporwave.toml`.

### `[interaction]`

| Key | Default | Notes |
|---|---|---|
| `selection` | REVERSED | a `bg` needs an explicit `fg` |
| `hover` | `raised` +1 step | `bg`-only is fine |
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

Colors only — the characters live in `[charts.glyphs]`, like every other
section. The bold/dim modifiers are the [monochrome contract](#monochrome-contract)
and come from code.

| Key | Default color |
|---|---|
| `positive` | green (BOLD in code) |
| `neutral` | terminal ink (DIM) |
| `negative` | red (BOLD) |
| `bar` | cyan |
| `track` | terminal ink (DIM) |
| `baseline` | ← `text.muted` dimmed |
| `label` | ← `text.muted` dimmed |

**`[charts.glyphs]`** — every chart character. Series fills: `positive` (`▓`),
`neutral` (`▓`), `negative` (`▓`). Count bars: `bar` (`▓`, filled), `track`
(`░`, empty). Signed-column furniture: `baseline` (`┈`, the zero-line tick in
the gaps/edges) and `rule` (`─`, the zero-line under each column). Diverging
(Δ / mood) bars: `diverge_track` (`·`, an empty cell) and `diverge_center`
(`│`, the center pivot). The eighths ramps for vertical bars: `ramp_up`
(`" ▁▂▃▄▅▆▇█"`, exactly 9 chars) and `ramp_down` (`" ▔▀█"`, exactly 4) — the
only multi-character keys. `eclipse` varies the series glyphs (`█ ▒ ░`) so the
three read apart without hue.

### `[markdown]`

| Key | Default |
|---|---|
| `heading` | terminal ink |
| `heading2` | ← `heading` |
| `subheading` | ← `heading` |
| `link` | underlined |
| `code` | terminal ink |
| `inline_code` | ← `code` |
| `blockquote` | terminal ink |
| `highlight` | `primary`, reversed + bold |

`heading` styles H1, `heading2` styles H2 (inheriting `heading` until you split them),
and `subheading` the faded H3–H6. (The renderer maps the levels this way — the markdown
parser only classifies them.) `code` is the fenced code block; `inline_code` the
`` `inline` `` spans, inheriting `code` until you give inline code its own look (e.g. a
background chip). `highlight` styles `==marked==` text.

**`[markdown.syntax]`** — fenced-code highlighting. Sixteen keys: `comment`,
`keyword`, `string`, `string_escape`, `number`, `constant`, `function`, `type`,
`variable`, `property`, `operator`, `punctuation`, `attribute`, `tag`, `label`,
`error`. An omitted key renders that category as plain code; omitting the table
disables highlighting.

**`[markdown.glyphs]`** — the reader's structural chrome. Short strings (not single
glyphs): `quote_rail` (`│ `, the blockquote rail), `code_rail` (`│ `, the fenced-code
rail), the code-fence frame `code_top` (`╭─`) / `code_bottom` (`╰─`), and the task-list
checkboxes `task_done` (`[x]`) / `task_todo` (`[ ]`) — set these to single boxes (`☑` /
`☐`) if your theme has the glyphs. Single characters: `bullet` (`-`, the unordered-list
marker; ordered lists keep `N.`). The `[markdown.glyphs.alert]` sub-table sets the icon
leading each GitHub alert — `note` (`i`), `tip` (`*`), `important` / `warning` /
`caution` (`!`); their band colors ride the status hues.

### `[metadata]` — the entry view's metadata section

**`[metadata.pills]`** — the feelings/people/activities/tags chips.

| Key | Form | Default |
|---|---|---|
| `style` | `reversed` \| `bg` \| `bracket` | `reversed` |
| `feelings` / `people` / `activities` / `tags` | style table | ← `interaction.hover` |

`reversed` inverts each chip's cell (the classic/e-ink look; the category
colors are ignored — part of the [monochrome contract](#monochrome-contract)).
`bg` fills chips with the per-category styles; a `bg`-only spec is fine (it
layers under the value's own ink, like hover). `bracket` renders plain
`[value]` text. Every style occupies the same cells, so switching themes never
moves a chip or its click target.

**`[metadata.environment]`** — the environment strip's air-quality badge, shown
only when the stored European AQI reaches 60: `aqi_poor` (60–79, ←
`status.warning`), `aqi_very_poor` (80–99, ← `status.error`),
`aqi_extremely_poor` (100+, ← `status.error`, BOLD in code). `pollen_high`
(← `status.warning`) styles the high-pollen badge, shown only when a stored
species count (birch/grass/ragweed) reaches its "high" band.

**`[metadata.glyphs]`** — the strip's characters: `rule` (`─`, the full-width
rule above the block), `separator` (`·`, the dot between strip items), `location`
(`⚑`), `sunrise` (`↑`), `sunset` (`↓`), `aqi` (`▲`), `pollen` (`❀`), plus the category glyph
leading each pill: `feelings` (`♥`), `people` (`@`), `activities` (`◆`),
`tags` (`#`). The mood bar's cells: `mood_fill` (`▓`) and `mood_track` (`░`) —
its center marker is the shared `[charts.glyphs]` `diverge_center`, and the heavy
at-zero variant stays code-side. Two per-slug tables: `[metadata.glyphs.weather]` (`clear` `☀`,
`mostly_clear` `☼`, `partly_cloudy` / `cloudy` `☁`, `fog` `≡`, `drizzle` /
`rain` `☂`, `snow` `❄`, `thunderstorm` `↯`) and `[metadata.glyphs.moon]`
(`new` `○`, `waxing_crescent` `☽`, `first_quarter` / `waxing_gibbous` `◐`,
`full` `●`, `waning_gibbous` / `last_quarter` `◑`, `waning_crescent` `☾`).
The retro themes — `classic` (also the built-in fallback), `crt`, `gameboy`,
`matrix`, `wasteland` — carry all-ASCII sets.

### `[toast]` / `[tabs]`

`[toast.glyphs]`: `edge` (`┃`, toast accents on flat chrome) and `progress` (`─`,
the dismissal countdown line along the bottom edge). `[tabs]` sets
`separator` (← `text.muted` dimmed); `[tabs.glyphs]`: `separator` (`·`).

### `[indicators.glyphs]`

Small state markers that ride the surrounding text style (glyphs only, no
colors of their own): `expanded` (`▾`) / `collapsed` (`▸`) — a group's
disclosure marker — and `starred` (`★`, trailing a starred entry).

## Monochrome contract

So meaning survives without color, some distinctions are enforced in code and
can't be themed away: positive and negative chart bars are bold (neutral ones dim),
muted ink is dim, the selection fallback is inverted, reversed metadata pills
ignore theme colors (their category glyphs still render, carrying the category
without hue), the worst air-quality band is bold, and chart series are set
apart by glyph as well as hue. A theme picks the colors and glyphs, but can never
flatten a positive value into plain body text.

`eclipse` is the reference case — pure black-and-white, where those enforced
modifiers and the three chart fill glyphs carry every distinction on their own.

## Chrome styles

- **Flat** — surfaces separate by background layer (`base` → `content` →
  `dialog`/`raised`), focused panels get a `focus_stripe` down the left edge, and
  the selection marker is `●`. Needs a theme with real surface colors.
- **Bordered** — drawn borders; focus reads through thick + bold strokes, and the
  selection marker is `>`.

Themes declare a preference in `chrome.default_style`; `[ui] chrome` (or `b` in
the picker) forces one everywhere.
