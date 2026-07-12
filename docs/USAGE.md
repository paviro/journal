# Usage

```bash
notema                       # launch the TUI
notema log "Had a good day"  # quick entry from the command line
notema log                   # compose an entry in the fullscreen editor
echo "note" | notema log     # entry from stdin
notema use personal          # set the default journal for new entries
```

Run `notema` with no arguments the first time to walk through setup: pick a journal
root (default `~/Journals`) and optionally enable encryption. Config is written to
`~/.config/notema/config.toml` (`~/Library/Application Support/de.paviro.notema/`
on macOS), overridable with `$XDG_CONFIG_HOME` or `--config <DIR>`.

## Logging with metadata

```bash
notema log "Shipped the release" \
  --journal work \
  --tag project,milestone \
  --person Alice --activity coding \
  --feeling proud,focused \
  --mood 3
```

`--feeling` takes values from a fixed vocabulary — see
[STORAGE-FORMAT.md](STORAGE-FORMAT.md#feelings-vocabulary).

## Location

Open the location dialog on an entry (`l`) to set where it was written: type a
place name, an address, or `lat, lon`. Addresses and coordinates are geocoded
through OpenStreetMap Nominatim. Once an entry has coordinates, Notema also fetches
the weather and air quality for that place and time from Open-Meteo.

Press `Ctrl+L` to grab the device's **current** location, reverse-geocoded to an
address. The provider is platform-specific and only ever produces
`latitude`/`longitude` — no IP-based fallback:

- **Android / Termux** — `termux-location`. Install the Termux:API app and
  `pkg install termux-api`.
- **Linux** — GeoClue2 over D-Bus. Its default Wi-Fi backend (Mozilla Location
  Service) was retired in 2024, so a machine with no GPS and no reconfigured
  backend (e.g. BeaconDB) reports "no location".
- **macOS** — CoreLocation. Since Ventura a bare CLI can't get location, so Notema
  extracts a small signed helper app to `~/Library/Application
  Support/de.paviro.notema/` on first use and reads the fix from it. Grant
  Location access when prompted, or later in System Settings → Privacy & Security →
  Location Services.

## Themes

`,` opens settings, then `t` opens the theme picker. Up/Down previews live, `b` cycles
chrome (default → flat → bordered), `m` cycles color mode (auto → dark → light;
hidden on themes without variants), Enter applies and saves, Esc reverts. Broken
themes are listed but can't be applied.

Edits to the **active** theme hot-reload (debounced ~400 ms). A broken edit shows
an error toast and keeps the current theme; the next valid save loads. Writing your
own themes is covered in [THEME-REFERENCE.md](THEME-REFERENCE.md); see
[THEMES.md](THEMES.md) for the bundled gallery.

## Import from Day One

```bash
notema import dayone ./export/personal.json --journal personal
notema import dayone ./export/personal.json --journal personal --download-images   # also fetch remote image links
```

Already-imported entries are skipped on re-run. Audio/video/PDF attachments are not
yet supported and are reported and skipped.
