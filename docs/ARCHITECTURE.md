# Architecture

Notema is a Cargo workspace. Reusable logic lives in `notema-*` library crates; the
application is the root package.

## Workspace crates

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
analytics. Import and context never depend on storage in their production
graph (test-only `dev-dependencies` on storage, for round-trip fixtures, are
exempt). FUSE reaches storage through its public facade.

Network access is a capability, not a layer: `notema-context` (geocoding,
weather, air quality) and `notema-storage` (remote asset download) open
sockets; every other crate — domain, analytics, encryption, import, fuse — must
not.

Errors follow the anyhow + thiserror split: the domain crates expose typed
`thiserror` enums, the application uses `anyhow` at its edges (`bail!`,
`.context()`, clean cause chains from `main`), and cross-crate handlers that
need to branch downcast the typed error back out of an `anyhow::Error` (e.g.
storage degrading an entry to a locked placeholder on an `EncryptionError`).
That downcast is a real contract: a crate changing its error type can break a
caller silently.

## Application flow

Keyboard and mouse handlers translate input into `Action` values. Only
`dispatch_action` mutates application *model* state; the feature reducers it
calls may mutate, but input translation may not. `DispatchOutcome` is the event
loop control result.

Rendering is a narrower rule than "never mutates": it may write view-derived
state only — clamp a scroll offset or selection to the content it just laid out
(so Home/End and a shrinking list stay in range), and record hit-test geometry
for the mouse layer to read next frame. It must never write model or lasting
navigation intent. Any such view-state a frame records (scroll geometry,
image/link hit maps) is reset at the top of the next frame so a stale rect can't
capture input for a panel that is no longer drawn there.

Panel focus is separate from row selection, and Reader and Insights scroll
independently of it. Render caches are keyed on the data, width, and theme
inputs that can invalidate them.

Dialogs share the action dispatcher. UI elements are clickable, but hover only
highlights that a row is clickable and never commits it.

## Persistence

Every persisted TOML document carries `schema_version = 1`, and unsupported
versions fail rather than being guessed. The documents are entry front matter,
config and state (root crate), themes (root crate), and the encryption roster,
pins, identity, and pending requests (`notema-encryption`, which owns the
`.age/` sub-layout). This is a clean-slate v1: pre-release data predating the
version field is intentionally unsupported, with no migrations.

Every persisted write goes through one fsync-ing atomic primitive
(`notema_encryption::atomic_write`: write a per-process temp, fsync, rename,
fsync the parent dir), so a crash can't truncate a good file.

Malformed entry front matter does not hide the body. The entry stays readable
with a warning, and the TUI blocks editing it (every metadata edit would touch
the unparseable front matter); storage still permits a byte-preserving
body-only write. An edit only rewrites the fields Notema owns; any other keys in
the front matter, including ones nested inside known tables, are left as they
were.

Config and theme files reject unknown keys: they are hand-edited, so a typo
should fail loudly rather than be silently ignored. Invalid machine-written
state is renamed aside and recreated.

## Platform code

Linux, Android, and macOS location providers are selected at compile time.
`notema-locate` ships only on macOS: a bare CLI binary can't obtain CoreLocation
authorization there, so the location code lives in a separate helper that ships
wrapped in a signed `.app`.

FUSE is an optional feature requiring libfuse3 headers and libraries; the
standard binary has no FUSE dependency. Unsafe Rust lives in exactly two places:
the C callback boundary in `notema-fuse` and the Objective-C bindings in
`notema-locate`. Every other crate carries `#![forbid(unsafe_code)]`, and the
workspace lints deny `unsafe_op_in_unsafe_fn` and undocumented unsafe blocks
everywhere.
