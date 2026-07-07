# journal

A terminal-first, markdown journaling app with a full TUI, optional end-to-end
encryption, and multi-device sync over any file syncing tool you already use
(Syncthing, Nextcloud, Dropbox, iCloud, …).

Entries are plain markdown files with TOML front-matter, organized by date on
disk. Nothing is locked into a proprietary format — your journal is just a
folder of `.md` files (or `.md.age` files once encryption is on). It runs
anywhere Rust runs, including Android via Termux, and is built to stay readable
on light, dark, and monochrome/e-ink terminals.

## Features

- **TUI** — three-pane browser (journals / entries / preview) with mouse and
  keyboard (arrows + vim keys) navigation, live rendered markdown, in-terminal
  image rendering, and entry metadata.
- **Fuzzy search** across the whole corpus, including metadata.
- **Rich metadata** per entry — tags, people, activities, feelings (from a
  fixed vocabulary), and a mood score (-5…+5).
- **Editor integration** — write and edit entries in `$EDITOR`.
- **Day One import** — import a Day One JSON export, photos included.
- **End-to-end encryption** — per-device [age](https://age-encryption.org) keys,
  a signed device roster, and an approval flow for adding new devices. Private
  keys never leave the device.

## Install

Requires a Rust toolchain (edition 2024).

```bash
cargo install --path .
# or
cargo build --release   # binary at target/release/journal
```

Cross-compilation targets (Termux/Android, musl, Windows, macOS universal, …)
are defined in `Makefile.toml` — e.g. `cargo make build-termux`.

## First run

Run `journal` with no arguments to start setup. It asks for:

- **Journal root** — the folder your entries live in (default `~/Journals`).
  Point this at a synced folder if you want multi-device sync (see below).
- **Editor** — command used to write entries (default `nano`).

For a brand-new, empty root it also offers to enable encryption on the spot.
Config is written to `~/.config/journal/config.toml`
(or `$XDG_CONFIG_HOME/journal`).

## Usage

```bash
journal                       # launch the TUI
journal log "Had a good day"  # quick entry from the command line
journal log                   # compose an entry in $EDITOR
echo "note" | journal log     # entry from stdin
journal use personal          # set the default journal for new entries via the cli
```

Attach metadata when logging:

```bash
journal log "Shipped the release" \
  --journal work \
  --tag project,milestone \
  --person Alice --activity coding \
  --feeling proud,focused \
  --mood 3
```

### Import from Day One

```bash
journal import dayone ./export/personal.json --journal personal
journal import dayone ./export/personal.json --journal personal --download-images   # also fetch remote image links
```

Already-imported entries are skipped on re-run. Audio/video/PDF attachments are
not yet supported and are reported and skipped.

## Storage layout

Entries are markdown files with TOML front-matter, bucketed by date:

```
<journal_root>/
├── personal/
│   └── 2026/07/05/2026-07-05T14-30-00-<id>.md          # entry (or .md.age when encrypted)
│       └── 2026-07-05T14-30-00-<id>.assets/            # images referenced by the entry
└── .age/                                               # created once encryption is on (see below)
```

```markdown
+++
created_at = "2026-07-05T14:30:00+02:00"
edited_at = "2026-07-05T14:30:00+02:00"
tags = ["work"]
feelings = ["focused"]
mood = 3
+++

# Entry body
Markdown content here.
```

Per-device settings and this device's private key live separately, in the config
directory (`~/.config/journal/`), and are **never** part of the journal folder.

The complete on-disk format — every front-matter field, the config/state files,
and how to recover encrypted entries with the standard `age` CLI — is documented
in [`docs/STORAGE-FORMAT.md`](docs/STORAGE-FORMAT.md).

## Sync

`journal` has no server and no sync of its own. Instead, point your **journal
root** at a folder managed by whatever file-sync tool you already run —
Syncthing, Nextcloud, Dropbox, iCloud Drive, etc. — and it appears on every
device. Each device runs its own `journal` binary against its own copy of that
folder.

If you use encryption, this is exactly the design: the synced folder carries
only ciphertext and public key material. Each device's **private key stays
local** and is never synced, so a compromised sync account cannot read your
entries. Encryption protects the *secrecy* of your journal, not its *authenticity* —
see [What encryption does and doesn't protect](#what-encryption-does-and-doesnt-protect).

## Encryption

Encryption is end-to-end and device-based, built on [age](https://age-encryption.org). 

Every device that can read the journal has its own keypair; entries are encrypted to all trusted devices. Adding a device
is an explicit approval step performed from a device that can already read the journal.

### What encryption does and doesn't protect

Encryption here is deliberately scoped — it's a serverless, single-owner design:

- **Protects:** entry and attachment *contents* are unreadable to anyone without a
  trusted device key, and the device roster is signed, so nobody can add a rogue
  device without the out-of-band fingerprint approval below.
- **Does not protect:** entries and attachments are **not signed** and carry no
  author attribution. Someone with **write** access to your synced folder can inject
  or replace entries and attachments without detection. (They still can't *read*
  anything, and they could equally just delete files.) The guarantee is the *secrecy*
  of your journal, not its *authenticity* against a tamperer who controls the sync
  medium.

### Enable encryption

On the first device:

```bash
journal encryption enable            # names the device, optionally sets a passphrase
```

This creates this device's key, records it as the first (genesis) entry in the
signed roster, and encrypts every existing plaintext entry. You'll be asked
whether to protect the key with a passphrase; pass `--no-passphrase` to skip
that prompt.

> **Back up your key.** `~/.config/journal/identity.toml` is the only thing that
> can decrypt this device's view of the journal. If you lose every trusted
> device's key, encrypted entries are unrecoverable.

### Add a new device

Adding a device is a request-and-approve handshake. It only works once the
synced folder (including `.age/`) has reached the new device — so **let your
sync tool finish syncing first**.

**1. On the new device — request access:**

```bash
journal encryption device enroll     # prompts for a device name and optional passphrase
```

This generates the new device's keypair and drops a `pending-<id>.toml` request
into the synced `.age/` folder. It prints the device's public recipient and a
**fingerprint** — keep this on screen (or note it down) for the next step. The
new device cannot read anything yet.

**2. Let it sync**, so the pending request reaches a device that can already
read the journal.

**3. On an existing (trusted) device — approve it.** There are two ways, and
both do the same thing:

- **In the TUI (easiest):** when a pending request has synced in, `journal`
  shows an approval modal at launch listing the requesting device's name and
  fingerprint. Approve or reject it right there.
- **On the command line:**

  ```bash
  journal encryption device list                 # see pending requests + fingerprints
  journal encryption device approve <name>       # or: --all
  ```

Before you approve, **compare fingerprints** (see below). Approving adds the
device to the signed roster and **re-encrypts every entry** so the new device
is now a recipient.

**4. Let it sync back.** Once the updated roster and re-encrypted entries reach
the new device, it can read and write the journal. (Unlock with its passphrase
if it set one.)

To reject a request instead:

```bash
journal encryption device reject <name>        # or: --all
```

#### Comparing fingerprints

The fingerprint is a short, human-readable summary of a device's public key. It
is the check that stops anyone with write access to your synced folder from
sneaking a rogue device onto the roster: a request is just a file, and it grants
nothing until a human approves it.

Approve **only** when the two fingerprints match:

- **On the new device**, the fingerprint is printed by `journal encryption
  device enroll` (and again by `journal encryption device list` there).
- **On the approving device**, the same fingerprint appears in the TUI approval
  modal and in `journal encryption device list`.

Confirm the two are identical **out-of-band** — read it aloud over a call,
compare it in person, or send it over a channel you already trust. Don't rely on
the synced folder itself to carry the fingerprint, since that's exactly the
channel an attacker would control. If the fingerprints don't match, reject the
request.

### Manage devices

```bash
journal encryption device list                 # trusted devices + pending requests
journal encryption device rename OLD NEW        # relabel a device (no re-encryption)
journal encryption device revoke <name>         # revoke a device and re-encrypt without it
journal encryption device rotate                # replace this device's key, retire the old one
journal encryption device passphrase            # add / change this device's key passphrase
journal encryption device passphrase --remove   # store the key unprotected
```

Revocation is **forward-only**: re-encryption excludes the revoked device from
future entries, but any entries it already synced remain readable to it. Rotate
a device's key (or revoke and re-enroll) if you suspect its key was exposed.

If encryption is disabled on one device (`journal encryption disable`), the
other devices notice on next launch, retire their local key material, and fall
back to reading the now-plaintext journal.

### Disable encryption

```bash
journal encryption disable            # decrypts every entry and turns encryption off
```

Destructive encryption operations prompt for confirmation; pass `-y`/`--yes` to
skip the prompt in scripts.

## Configuration

`~/.config/journal/config.toml`:

```toml
journal_root = "~/Journals"     # point at a synced folder for multi-device use
editor = "nano"
default_journal = "personal"
download_remote_images = true   # fetch remote image links referenced in entries
show_hints = true
show_journals = true
```

Use `--config <DIR>` to run against an alternate config directory (it must be a
directory, not a file).

## License

See [`LICENCE`](LICENCE) (EUPL v1.2).
