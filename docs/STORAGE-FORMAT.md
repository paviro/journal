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
— they live in the config directory (`~/.config/journal/`, see below) and are
never synced.

## Entry file

An entry is a UTF-8 Markdown file: a TOML **front matter** block fenced by `+++`
lines, a blank line, then the Markdown body.

```markdown
+++
created_at = "2026-07-05T14:30:00+02:00"
edited_at = "2026-07-05T14:30:00+02:00"
tags = ["work", "release"]
people = ["Alice"]
activities = ["coding"]
feelings = ["focused", "proud"]
mood = 3
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

| Key            | Type            | Meaning |
|----------------|-----------------|---------|
| `created_at`   | RFC 3339 string | When the entry was created. If missing or unparseable, the app falls back to the date in the filename. |
| `edited_at`    | RFC 3339 string | Last time a human edited the entry (its body or metadata). It is **not** touched by encryption, re-encryption, or asset rewrites — only genuine edits move it. |
| `tags`         | array of string | Free-form tags. |
| `people`       | array of string | Free-form people references. |
| `activities`   | array of string | Free-form activities. |
| `feelings`     | array of string | From a fixed vocabulary (below); unknown values are dropped on read. |
| `mood`         | integer         | Overall mood, clamped to `-5..=5`. Out-of-range or non-integer values are dropped to “no mood” rather than failing the parse. |
| `import_id`    | string          | Provenance of an imported entry, as `source:id` (e.g. `dayone:<UUID>`). Absent for entries created in the app. Used to skip re-importing. |

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
`~/.config/journal/`) and are never synced. Unknown keys are ignored, so new
options stay backward-compatible.

`config.toml` — user-authored settings:

```toml
journal_root = "~/Journals"       # tilde is expanded on load
editor = "nano"
default_journal = "work"          # optional
show_hints = true
show_journals = true
download_remote_images = true
```

`state.toml` — machine-written session state (kept separate so it never clutters
your settings):

```toml
last_journal = "personal"         # journal reselected on next launch
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
- `~/.config/journal/identity.toml` — **this device's private keys** (never
  synced). A TOML file holding `device_name` plus either `plain_keys` (mode-0600
  cleartext when no passphrase) or `encrypted_keys` (age ASCII armor when a
  passphrase is set). Inside is a small bundle: `x25519` (the age secret key) and
  `ed25519` (the signing seed, hex).
- `~/.config/journal/devices-trust.toml` — this device's local trust pins
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
