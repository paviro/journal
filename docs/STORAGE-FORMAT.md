# Storage format

`journal` stores everything as plain files on disk — no database, no proprietary
container. This document specifies that on-disk format so other tools can read and
write entries, and so you can recover your data with standard utilities even
without this app. The format is stable as of the first release; fields are only
ever added in backward-compatible ways.

## Directory layout

Entries live under the **journal root** (the folder you chose at setup, e.g.
`~/Journals`), bucketed by journal name and date:

```
<journal_root>/
├── personal/                                       # one directory per journal
│   └── 2026/07/05/                                 # YYYY / MM / DD of creation
│       ├── 2026-07-05T14-30-00-<id>.md             # an entry (.md.age when encrypted)
│       └── 2026-07-05T14-30-00-<id>.assets/        # images/files referenced by that entry
└── .age/                                           # present only when encryption is on
    ├── devices.toml                                # signed device roster
    └── pending-<id>.toml                           # join requests awaiting approval
```

- Entry filename: `<YYYY-MM-DDTHH-MM-SS>-<id>.md`, where `<id>` is a 12-character
  [nanoid](https://github.com/ai/nanoid). The timestamp encodes the creation time.
- When encryption is enabled, entry files are `age`-encrypted and named `.md.age`.
- Per-entry assets sit in a sibling directory named `<entry-stem>.assets/`.

Per-device settings and this device's private key are **not** in the journal root
— they live in the config directory (`~/.config/notema/`, see below) and are
never synced.

## Entry file

An entry is a UTF-8 Markdown file: a TOML **front matter** block fenced by `+++`
lines, a blank line, then the Markdown body.

```markdown
+++
activities = ["coding"]
feelings = ["focused", "proud"]
people = ["Alice"]
tags = ["work", "release"]
mood = 3
starred = true

[datetime]
created_at = "2026-07-05T14:30:00+02:00"
edited_at = "2026-07-05T14:30:00+02:00"
timezone = "Europe/Berlin"
writing_seconds = 320

[import]
source = "dayone"
id = "UUID"

[location]
name = "Twin Peaks"
house_number = "501"
road = "Twin Peaks Blvd"
suburb = "Twin Peaks"
postcode = "94114"
city = "San Francisco"
county = "San Francisco County"
state = "California"
country = "United States"
latitude = 37.7544
longitude = -122.4477

[weather]
condition = "partly-cloudy"
temperature_celsius = 19.9
feels_like_celsius = 19.5
dew_point_celsius = 12.4
humidity = 0.62
pressure_mb = 1013.2
cloud_cover = 0.4
visibility_km = 12.5
precipitation_mm = 0.0
wind_speed_kph = 12.0
wind_gust_kph = 28.0
wind_direction = 210.0
source = "Open-Meteo"

[air_quality]
european_aqi = 42
us_aqi = 55
pm2_5 = 12.4
pm10 = 18.0
carbon_monoxide = 210.0
nitrogen_dioxide = 14.2
ozone = 68.0
sulphur_dioxide = 2.1
uv_index = 6.2
birch_pollen = 0.0
grass_pollen = 24.0
ragweed_pollen = 0.0
source = "Open-Meteo"

[celestial]
moon_phase = 0.5
moon_phase_name = "full"
sunrise = "2026-07-05T05:51:00+02:00"
sunset = "2026-07-05T21:29:00+02:00"
day_length_seconds = 56280
+++

# Entry body

Markdown content here.
```

- The block starts with a line that is exactly `+++` and ends at the next line that
  is exactly `+++`. Both LF and CRLF line endings are accepted.
- A single blank line separates the closing fence from the body.
- A file with no `+++` front matter is treated as all body, no metadata.
- Parsing is lenient: unknown keys are ignored, and any field may be omitted.
  Malformed front matter is treated as empty metadata rather than failing the file.

### Front matter fields

The flattened user metadata comes first (TOML requires scalars before tables),
then the system/import tables.

| Key            | Type            | Meaning |
|----------------|-----------------|---------|
| `activities`   | array of string | Free-form activities. |
| `feelings`     | array of string | From a fixed vocabulary (below); unknown values are dropped on read. |
| `people`       | array of string | Free-form people references. |
| `tags`         | array of string | Free-form tags. |
| `mood`         | integer         | Overall mood, clamped to `-5..=5`. Out-of-range or non-integer values are dropped to “no mood” rather than failing the parse. |
| `starred`      | boolean         | Whether the entry is flagged as a favorite. Omitted when false. |
| `[datetime]`   | table           | `created_at` (RFC 3339; falls back to the filename date if missing), `edited_at` (RFC 3339; only genuine human edits move it — not encryption or asset rewrites), `timezone` (IANA zone name the entry was authored in, e.g. `Europe/Berlin` — capture-only, complements the offset in `created_at`), and `writing_seconds` (accumulated editor-open time, whole seconds; seeded from Day One's `editingTime` and grown by native edits that change the body). |
| `[import]`     | table           | Provenance of an imported entry: `source` (e.g. `dayone`) and `id` (the source's identifier). Absent for entries created in the app. Used to skip re-importing. |
| `[location]`   | table           | Where the entry was written. Fields are OpenStreetMap / Nominatim address keys, stored one-to-one — optional `name` (a place/venue label), `house_number`, `road`, `neighbourhood`, `quarter`, `suburb`, `borough`, `city_district`, `city`, `town`, `village`, `municipality`, `hamlet`, `postcode`, `county`, `state_district`, `province`, `region`, `state`, `country`, `latitude`, `longitude`. Only the keys a geocode returns are stored (the rest omitted). Two more are set only when the coordinates come from a device "grab GPS": `accuracy_m` (horizontal accuracy in metres) and `source` (the provider slug, e.g. `corelocation`, `geoclue`, `termux`). Set in the app via the location dialog; Day One import maps its coarse placemark onto `name`/`city`/`state`/`country`. Displayed but not searched. |
| `[weather]`    | table           | Weather at the time of writing — fetched from Open-Meteo when a location is set, or captured on Day One import. All optional: `condition` (a slug, e.g. `partly-cloudy`), `temperature_celsius`, `feels_like_celsius`, `dew_point_celsius`, `humidity` (0–1), `pressure_mb`, `cloud_cover` (0–1), `visibility_km`, `precipitation_mm`, `wind_speed_kph`, `wind_gust_kph`, `wind_direction` (degrees), and `source` (provider, for attribution). Capture-only, stored not surfaced. |
| `[air_quality]`| table           | Air quality and UV at the time of writing — fetched from Open-Meteo's air-quality endpoint (a separate provider than `[weather]`, so an entry may carry one without the other). All optional: `european_aqi`, `us_aqi`, `pm2_5`, `pm10` (µg/m³), `carbon_monoxide`, `nitrogen_dioxide`, `ozone`, `sulphur_dioxide` (µg/m³), `uv_index`, `birch_pollen`, `grass_pollen`, `ragweed_pollen` (grains/m³, Europe only), and `source` (provider, for attribution). Capture-only. |
| `[celestial]`  | table           | Sun/moon at the time of writing — computed locally when a location is set, or captured on Day One import. All optional: `moon_phase` (0–1), `moon_phase_name`, `sunrise`, `sunset`, `day_length_seconds` (sunset − sunrise). Capture-only. |

All list fields are plural; timestamps are RFC 3339 with an offset. There is no
schema-version field — the format evolves by adding optional fields, and readers
should ignore anything they don't recognize.

### Feelings vocabulary

`feelings` values are normalized to lowercase and must be one of:

```
calm, content, grateful, hopeful, joyful, excited, energized, focused, proud,
relieved, curious, okay, mixed, tired, bored, sad, lonely, anxious, stressed,
overwhelmed, frustrated, angry, guilty, numb
```

Values outside this list are silently dropped when an entry is read.

## Config and state files

Both live in the config directory (`$XDG_CONFIG_HOME/journal/` or
`~/.config/notema/`) and are never synced. Unknown keys are ignored, so new
options stay backward-compatible.

`config.toml` — user-authored settings:

```toml
[journal]
path = "~/Journals"               # tilde is expanded on load
default = "work"                  # optional

[editor]
command = "nano"

[attachments]
download_remote_images = true

[ui.layout.entry_viewer]
body_center_vertically = true     # center a short entry when it fits, no scrollbar
body_max_width = 100              # cap the body width in cells (0 = no cap)
```

`state.toml` — machine-written session state (kept separate so it never clutters
your settings), including UI toggles flipped from inside the TUI:

```toml
last_journal = "personal"         # journal reselected on next launch

[ui]
show_hints = true
show_journals = true
```

## Encryption files

When encryption is on, only ciphertext and public key material live in the synced
`.age/` folder; private keys never leave the device.

- `.age/devices.toml` — the **signed device roster**: an append-only log of
  Ed25519-signed `[[operation]]` entries (`genesis`/`add`/`revoke`/`rename`) naming
  each device's age public key (`enc_key`), signing public key (`sign_key`), and
  label. Verified from the genesis on every read; a tampered or rolled-back log is
  rejected wholesale.
- `.age/pending-<id>.toml` — a join request from a not-yet-approved device.
- `~/.config/notema/identity.toml` — **this device's private keys** (never
  synced). A TOML file holding `device_name` plus either `plain_keys` (mode-0600
  cleartext when no passphrase) or `encrypted_keys` (age ASCII armor when a
  passphrase is set). Inside is a small bundle: `x25519` (the age secret key) and
  `ed25519` (the signing seed, hex).
- `~/.config/notema/devices-trust.toml` — this device's local trust pins
  (genesis + last-seen head hash), never synced.

## Emergency recovery with the `age` CLI

If you ever need to read encrypted entries without this app, you can with the
standard [`age`](https://age-encryption.org) tool. Everything hinges on the age
secret key inside `identity.toml`.

**1. Get your age secret key** (`AGE-SECRET-KEY-1…`) out of `identity.toml`:

- *No passphrase* — the file contains a `plain_keys` block; read the `x25519`
  value directly. Put it in a keyfile:

  ```bash
  # copy the AGE-SECRET-KEY-1... value from identity.toml into key.txt
  printf 'AGE-SECRET-KEY-1...\n' > key.txt
  ```

- *Passphrase-protected* — the file contains an `encrypted_keys` block that is a
  standalone age file in ASCII armor (`-----BEGIN AGE ENCRYPTED FILE-----`). Copy
  that whole armored block (fences included) into `bundle.age`, then decrypt it
  with your passphrase to reveal the `x25519` secret key:

  ```bash
  age --decrypt bundle.age            # prompts for the passphrase, prints the key bundle
  ```

  Copy the resulting `AGE-SECRET-KEY-1…` line into `key.txt` as above.

**2. Decrypt an entry** with that key:

```bash
age --decrypt --identity key.txt \
  "personal/2026/07/05/2026-07-05T14-30-00-<id>.md.age"
```

The output is the plaintext `.md` entry — front matter and body — exactly as
documented above. The same key decrypts every entry the journal encrypted to this
device.
