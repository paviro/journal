# Storage format

Entries are plain files on disk — no database, no proprietary container. Any tool
can read and write them, and the standard `age` CLI can recover encrypted entries
without this app.

## Journal layout

```
<journal_root>/
├── .notema-store.toml                         # stable id for this synced root
├── personal/                                  # one directory per journal
│   ├── .journal.toml                          # per-journal id + optional theme
│   └── 2026/07/05/                            # YYYY/MM/DD of creation
│       ├── 2026-07-05T14-30-00-<id>.md        # entry (.md.age when encrypted)
│       └── 2026-07-05T14-30-00-<id>.assets/   # files referenced by the entry
├── work.archived/                             # archived journal (dir renamed)
├── .trash/                                    # deleted entries and journals
│   └── personal/2026/07/05/…                  # mirrors the journal tree
└── .age/                                      # only when encryption is on
    ├── devices.toml                           # signed device roster
    └── pending-<id>.toml                      # join requests awaiting approval
```

- Filename: `<YYYY-MM-DDTHH-MM-SS>-<id>.md`. `<id>` is a 4-character random
  alphanumeric suffix; the timestamp is the creation time.
- Encrypted entries are `age`-encrypted and named `.md.age`.
- Each entry's assets sit in a sibling `<entry-stem>.assets/` directory.
- A journal is archived by appending `.archived` to its directory name; the name
  without the suffix is what's shown. The entries inside are untouched.
- Deleting in the app moves the entry (with its assets) into
  `.trash/<journal>/…`, keeping its path so it can be restored; deleting a whole
  journal moves the directory to `.trash/<journal>`.

Per-device settings, the entry cache, and this device's private key live in the
config directory (`~/.config/notema/`), never in the journal root and never
synced. The cache is `library-cache.msgpack` for plaintext journals and
`library-cache.msgpack.age` for encrypted journals. iSH also stores the selected
root's binding in `ish-store.toml` there.

`.notema-store.toml` contains a random stable id used to identify the synced root.
It stays plaintext when entry encryption is enabled.

## Journal sidecar

Each journal folder holds a `.journal.toml`, created on first sight:

```toml
schema_version = 1
id = "a1b2c3d4"        # stable random handle, survives renames/archiving

[theme]                # optional; absent means the journal follows the global theme
name = "gameboy"
color_mode = "dark"    # auto | dark | light
chrome = "flat"        # default | flat | bordered
```

This file lives in the journal folder, so it syncs with the journal (config and
state don't). It's plaintext even on an encrypted store. `state.toml`'s
`last_journal_id` references the `id`, so the remembered journal survives folder
renames. The `[theme]` table is written and cleared as one unit; a device that
doesn't recognize a value uses its own config for that field. A corrupt or
unknown-version sidecar is left alone, and the journal falls back to the global
theme.

## Entry file

UTF-8 Markdown: a TOML front-matter block fenced by `+++`, a blank line, then the
body.

```markdown
+++
schema_version = 1

[entry]
tags = ["work", "release"]
feelings = ["focused", "proud"]
people = ["Alice"]
activities = ["coding"]
mood = 3
starred = true

[time]
created_at = "2026-07-05T14:30:00+02:00"
edited_at = "2026-07-05T14:30:00+02:00"
timezone = "Europe/Berlin"
writing_seconds = 320

[location]
name = "Tempelhofer Feld"
road = "Tempelhofer Damm"
city = "Berlin"
state = "Berlin"
country = "Germany"
latitude = 52.4730
longitude = 13.4050

[weather]
condition = "Partly cloudy"
temperature_celsius = 21.4
feels_like_celsius = 21.0
dew_point_celsius = 12.3
humidity = 0.58              # 0–1 fraction
pressure_mb = 1013.2
cloud_cover = 0.40           # 0–1 fraction
visibility_km = 24.0
precipitation_mm = 0.0
wind_speed_kph = 11.5
wind_gust_kph = 23.0
wind_direction = 245.0       # compass bearing the wind blows from
source = "open-meteo"

[air_quality]
european_aqi = 32
us_aqi = 41
pm2_5 = 6.8                  # µg/m³
pm10 = 12.1
carbon_monoxide = 142.0
nitrogen_dioxide = 9.4
ozone = 78.0
sulphur_dioxide = 1.2
uv_index = 5.4
grass_pollen = 18.0          # grains/m³, Europe only
birch_pollen = 0.0
ragweed_pollen = 0.0
source = "open-meteo"

[celestial]
moon_phase = 0.62            # 0–1 cycle fraction
moon_phase_name = "Waning gibbous"
sunrise = "2026-07-05T05:03:00+02:00"
sunset = "2026-07-05T21:28:00+02:00"
day_length_seconds = 59100

[import]
source = "dayone"
id = "UUID"
+++

# Entry body

Markdown content here.
```

Parsing rules:

- The front matter is delimited by two fence lines, each containing only `+++`
  (LF or CRLF line endings). One blank line then separates the closing fence from
  the body.
- A file with no front matter is treated as all body.
- `schema_version = 1` is required; any other or missing value is unsupported.
- Unknown keys survive edits. Optional fields may be omitted.
- Malformed front matter still shows the body, with a warning, but editing the
  entry in the app is blocked until you fix it by hand.

### Fields

`schema_version` is the one top-level key; everything else lives in a `[table]`.
The `[entry]` table holds what you write about the entry; the rest is
app-captured context. Empty fields (and empty tables) are omitted.

| Key | Type | Meaning |
|---|---|---|
| `schema_version` | int | Required; `1`. |
| `[entry]` | table | Your own metadata: `tags`/`people`/`activities` (free-form string[]), `feelings` (string[], fixed vocabulary below; unknown values dropped on read), `mood` (int clamped to `-5..=5`; invalid becomes "no mood"), `starred` (bool, omitted when false). |
| `[time]` | table | `created_at`, `edited_at` (RFC 3339; `edited_at` moves only on real body/metadata edits, not on re-encryption), `timezone` (IANA zone name), `writing_seconds` (accumulated editor-open time). |
| `[location]` | table | OpenStreetMap / Nominatim address keys (`name`, `road`, `city`, `state`, `country`, …) plus `latitude`, `longitude`. A device GPS grab also sets `accuracy_m` and `source`. Displayed, not searched. |
| `[weather]` | table | Captured from Open-Meteo when a location is set: `condition`, temperatures, `humidity`, `pressure_mb`, wind, precipitation, `source`. Capture-only. |
| `[air_quality]` | table | Open-Meteo air-quality endpoint: `european_aqi`/`us_aqi`, `pm2_5`/`pm10`, gases, `uv_index`, pollen (Europe), `source`. Capture-only. |
| `[celestial]` | table | Computed locally: `moon_phase`, `moon_phase_name`, `sunrise`, `sunset`, `day_length_seconds`. Capture-only. |
| `[import]` | table | Imported entries only: `source` (e.g. `dayone`) and `id`. Used to skip re-imports. |

### Feelings vocabulary

Normalized to lowercase and matched against a fixed vocabulary (plus per-feeling
aliases); values outside it are dropped on read. The canonical list lives in
[`crates/notema-domain/src/feelings.rs`](../crates/notema-domain/src/feelings.rs).

## Config and state

Both live in the config directory (`$XDG_CONFIG_HOME/notema/` or
`~/.config/notema/`, `~/Library/Application Support/de.paviro.notema/` on macOS),
require `schema_version = 1`, reject unknown keys, and are never synced.

`config.toml` — user settings:

```toml
schema_version = 1

[journal]
path = "~/Journals"             # tilde expanded on load
default = "work"                # optional

[editor]
start_fullscreen = false

[attachments]
download_remote_images = true

[location]
use_location_timezone = true    # stamp a located new entry with its place's timezone, not the system's

[ui]
theme = "journal"               # global theme; theme file in <config>/themes/, without .toml
color_mode = "auto"             # auto | dark | light
chrome = "default"              # default | flat | bordered
ignore_journal_themes = false   # true: this device ignores per-journal themes, uses `theme`

[ui.layout.reader]
body_center_vertically = true
body_max_width = 100            # cells; 0 = no cap
show_link_urls = false
```

`state.toml` — machine-written session state, kept out of your settings:

```toml
schema_version = 1
last_journal_id = "a1b2c3d4"    # the journal sidecar id, so it survives renames

[ui]
show_hints = true
show_journals = true
```

## Encryption files

Encryption splits its state in two: public material that syncs with the journal,
and private keys that never leave the device.

**Synced, in the journal's `.age/` folder** (public only):

- `devices.toml` — the signed device roster: an append-only log of Ed25519-signed
  `genesis`/`add`/`revoke`/`rename` operations, each naming a device's age public
  key, signing key, and label. Verified from genesis on every read; a tampered or
  rolled-back log is rejected whole.
- `pending-<id>.toml` — a join request from a not-yet-approved device.

**Local, in the config directory** (never synced):

- `identity.toml` — this device's private keys: `device_name` plus either
  `plain_keys` (mode-0600 cleartext) or `encrypted_keys` (age armor when a
  passphrase is set). Holds `x25519` (the age secret) and `ed25519` (the signing
  seed).
- `devices-trust.toml` — local trust pins (genesis + last-seen head hash).

Decrypting entries by hand with the `age` CLI is covered in
[`docs/ENCRYPTION.md`](ENCRYPTION.md#recovery-without-the-app).
