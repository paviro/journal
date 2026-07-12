# Architecture

Notema is a workspace because its reusable policy-free layers have different
dependency and platform boundaries. The executable, CLI, and TUI stay in the
root package; they are one application and share navigation, configuration,
background work, and presentation state.

## Workspace boundaries

| Package | Owns | Must not own |
|---|---|---|
| `notema-domain` | Entry values, validated coordinates, feelings, search value types, Markdown link parsing | Filesystem access, network access, terminal types |
| `notema-analytics` | Pure aggregation over borrowed domain entries | I/O, clocks, rendering |
| `notema-context` | Geocoding, weather, air quality, celestial calculations, device location | Storage or TUI state |
| `notema-import` | Parsing and normalizing external export formats | Journal creation, dedup policy, writes |
| `notema-encryption` | Keys, recipients, signed roster, ciphertext and identity formats | Journal paths and entry layout |
| `notema-storage` | Journal layout, entry codecs, assets, atomic writes, encryption orchestration | CLI prompts or terminal UI |
| `notema-fuse` | libfuse adapter and mount path policy | Entry parsing and business rules |
| `notema-seed` | Development corpus generation | Production command surface |
| `notema-locate` | macOS CoreLocation helper executable | Application state |
| root `notema` | CLI use cases, config/state, TUI, rendering, workers | Reusable domain or storage primitives |

Dependencies point toward `notema-domain`. `notema-storage` may use
`notema-encryption`; the application composes storage, import, context, and
analytics. Import and context never depend on storage. FUSE reaches storage
through its public facade.

The current crate count is deliberate. Splitting the TUI into widget, app, or
media crates would expose internal state without creating an independent reuse
or platform boundary. Combining encryption with storage would make its path-free
cryptographic API harder to audit. Combining analytics with domain would mix
derived reports into the persisted model.

## Application flow

Keyboard and mouse handlers translate input into `Action` values. Only
`dispatch_action` applies application mutations. Feature reducers called by the
dispatcher may mutate state; input translation and rendering may not create a
second mutation path. `DispatchOutcome` is the event loop control result.

The TUI keeps panel focus separate from row selection. Journals and entries form
a visible selection trail toward Preview. Preview and Insights scrolling is
independent from list selection. Render caches are keyed by the data, width, and
theme inputs that can invalidate them.

Dialogs use the same action dispatcher for keyboard and pointer input. List rows,
buttons, text fields, scrollbars, and editor prompts are intentionally clickable;
hover may preview a row but must not commit it.

## Persistence

Every persisted TOML document has `schema_version = 1`: entries, config, state,
themes, identities, pending requests, rosters, and trust pins. Unsupported
versions fail instead of being guessed. There is no migration layer before the
first release.

Malformed entry front matter does not hide the body. The entry is readable with
a warning; body-only edits preserve the raw front matter, while metadata edits
are blocked. Valid metadata edits retain unknown TOML keys recursively.

Config and theme files reject unknown keys because they are user-authored policy
documents where a typo should fail loudly. Invalid machine-written state is
renamed aside and recreated.

## Platform code

Linux, Android, and macOS location providers are selected at compile time.
`notema-locate` is bundled only for macOS. FUSE is an optional application
feature and requires libfuse3 headers and libraries; the standard binary has no
FUSE dependency.

Unsafe Rust is confined to `notema-fuse`, at the C callback boundary. The rest
of the workspace forbids unsafe code.
