<p align="center">
  <img src=".github/assets/themes.svg" alt="Notema cycling through its bundled themes" width="800">
</p>

<h1 align="center">Notema&nbsp;間</h1>

<p align="center">
  <i>A terminal-based Markdown journaling app with optional end-to-end encryption and multi-device sync.</i>
</p>

<p align="center">
  <a href="https://github.com/paviro/notema/releases"><img alt="latest release" src="https://img.shields.io/github/v/release/paviro/notema?style=flat-square&label=release"></a>
  <a href="https://github.com/paviro/notema/actions/workflows/ci.yml"><img alt="continuous integration" src="https://img.shields.io/github/actions/workflow/status/paviro/notema/ci.yml?branch=main&style=flat-square&label=CI"></a>
  <a href="https://interoperable-europe.ec.europa.eu/collection/eupl/eupl-text-eupl-12"><img alt="license" src="https://img.shields.io/badge/license-EUPL--1.2-blue?style=flat-square"></a>
</p>

<hr>

**Notema** (/noʊˈtɛ.mɑː/) combines *note* with the Japanese concept of 間 (*ma*) —
the pause, interval, or space between things, in time as well as on the page. Sync
runs through file-syncing tools you already use, such as Syncthing, Nextcloud, or
Dropbox.

Entries are plain markdown files with TOML front-matter, organized by date on
disk. Nothing is locked into a proprietary format — your journal is just a folder
of `.md` files (or `.md.age` files once encryption is on). It runs anywhere Rust
runs, including Android via Termux, and stays readable on light, dark, and
monochrome/e-ink terminals.

## Features

- **TUI** — three-pane browser (journals / entries / reader) with mouse and
  keyboard (arrows + vim keys) navigation, live rendered markdown, in-terminal
  image rendering, and entry metadata.
- **Fuzzy search** across the whole corpus, including metadata.
- **Themes** — TOML theme files with flat or bordered chrome, dark/light variants,
  a live-preview picker, and hot reload on edit.
- **Rich metadata** per entry — tags, people, activities, feelings, a mood score
  (-5…+5), and a location. Located entries also record the weather, air quality,
  and sun/moon data for that place and time.
- **Built-in editor** — write and edit entries in a fullscreen editor with markdown
  syntax highlighting.
- **Day One import**, photos and metadata included.
- **End-to-end encryption** — per-device [age](https://age-encryption.org) keys, a
  signed device roster, and an approval flow for new devices. Private keys never
  leave the device.
- **Decrypted FUSE mount** — browse an encrypted journal as a writable, decrypted
  filesystem.

## Install

Download a binary for your platform from the
[releases page](https://github.com/paviro/notema/releases), or build from source —
see [`docs/BUILDING.md`](docs/BUILDING.md).

## Quick start

```bash
notema                       # launch the full TUI (walks through setup on first run)
notema log "Had a good day"  # quick entry from the command line
notema log                   # compose an entry in the fullscreen editor
```

Notema has no server of its own — keep your journal on one machine, or sync its
folder with any file-sync tool you like and it appears on all your devices. With
encryption on, that synced folder holds only ciphertext and public keys; each
device's individual private key stays local, so whoever hosts the sync only ever
sees ciphertext and can't read your entries.

## Documentation

- [Usage](docs/USAGE.md) — logging with metadata, location capture, Day One import
- [Mobile (iOS / Android)](docs/MOBILE.md) — running under iSH and Termux, sync setup
- [Encryption](docs/ENCRYPTION.md) — setup, adding devices, key management
- [Mounting (FUSE)](docs/FUSE.md) — browse an encrypted journal as a filesystem
- [Storage format](docs/STORAGE-FORMAT.md) — on-disk layout, front-matter, config, recovery
- [Themes](docs/THEMES.md) — gallery of the bundled themes
- [Theme reference](docs/THEME-REFERENCE.md) — theme file format and the full token reference
- [Building](docs/BUILDING.md) — from source, cross-compile, FUSE builds
- [Architecture](docs/ARCHITECTURE.md) — crate layout and dependency rules
- [Development](docs/DEVELOPMENT.md) — checks, cross-builds, running the benchmarks

## License

See [`LICENCE`](LICENCE) (EUPL v1.2).

## Attribution

- **Weather and air quality data** from [Open-Meteo](https://open-meteo.com), under
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
- **Location geocoding** — © [OpenStreetMap](https://www.openstreetmap.org/copyright)
  contributors, via [Nominatim](https://nominatim.org), under the
  [ODbL](https://opendatacommons.org/licenses/odbl/).

Run `notema licenses` to print these credits with the full license texts of every
third-party Rust dependency.

## Disclaimer

This was built for personal use and relies heavily on AI-generated code. While I've
tested everything and use it daily, I take no responsibility for any issues you might
encounter. Use at your own risk.
