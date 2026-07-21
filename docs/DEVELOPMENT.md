# Development

## Setup

Needs the toolchain pinned in `rust-toolchain.toml` (Rust 1.96, with `clippy`
and `rustfmt`). `rustup` installs it automatically on first `cargo` invocation
in the repo.

Enable the versioned Git hooks once per clone:

```bash
git config core.hooksPath .githooks
```

The commit-message hook requires `type(scope): description`. Add `!` before the
colon for a breaking change: `type(scope)!: description`. CI checks the same
format for every commit pushed to `main` or included in a pull request.

## Checks

These mirror the CI jobs; run them before pushing.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

CI runs `cargo audit --deny warnings` as well; local runs need
`cargo install cargo-audit`. Acknowledged advisories are listed in
[AUDIT.md](AUDIT.md).

The `fuse` feature is off by default. To check the mount wiring compiles:

```bash
cargo check -p notema --features fuse --locked   # needs libfuse3 headers
```

## Cross-compiled builds

Official releases are built by CI on version tags (see
[RELEASING.md](RELEASING.md)). For local cross-builds, `Makefile.toml` provides
per-target tasks through
[`cargo-make`](https://github.com/sagiegurari/cargo-make):

```bash
cargo make build-termux            # Android/Termux ARM64
cargo make build-x86-gnu           # x86_64 Linux (glibc)
cargo make build-macos-universal   # Intel + Apple Silicon
cargo make build-windows-gnu       # Windows x86_64
cargo make run-tests               # full workspace test suite
```

See [BUILDING.md](BUILDING.md) for prerequisites and the FUSE variants.

## Seeding development data

`notema-seed` is a dev-only tool (outside the shipped binary's dependency graph)
that fills a store with generated entries — handy for exercising the TUI or the
benchmarks against a realistic corpus. It writes into a real store, so point it
at a throwaway directory, not your journal:

```bash
cargo run -p notema-seed -- \
  --root /tmp/notema-dev/journals \
  --config-dir /tmp/notema-dev \
  --count 750
```

`--journal` names the journal to fill (default `Sample`), `--days` spreads the
creation dates, and `--seed <n>` makes the data set reproducible. Run
`cargo run -p notema-seed -- --help` for the full list.

## Benchmarks

Benchmarks are deterministic and run as plain timed binaries (`harness = false`)
over 1k / 10k / 25k corpora, printing one line per size. They exist to catch
performance regressions on the paths that scale with journal size.

| Bench | Crate | Covers |
|---|---|---|
| `analytics` | `notema-analytics` | Cadence/mood/correlation aggregation |
| `scan` | `notema-storage` | Full journal scan: walk + parse + preview + haystack |
| `tui` | root (`--features bench`) | Full-frame render and in-memory fuzzy search |

```bash
cargo bench -p notema-analytics --bench analytics
cargo bench -p notema-storage --bench scan
cargo bench --features bench --bench tui
```

The `tui` bench reaches otherwise-private TUI paths through the `bench` feature,
which exposes a small `notema::bench` module (a `BenchApp` handle plus
`draw_frame`/`search`). The feature is dev-only and never compiled into the
shipped binary.

Reading the numbers: each line is the mean wall-clock time per iteration, e.g.
`scan/25000: 1.1s`. There is no built-in baseline comparison — record the 25k
numbers before a change and compare after. Treat a **>10% regression on any 25k
figure** as a regression to investigate, matching the workspace's performance
budget. The 25k row is the one that matters; the 1k/10k rows mostly surface
fixed per-run overhead.

Numbers are machine-relative; compare runs on the same hardware, ideally with a
quiet system and on the `release`/`bench` profile (which `cargo bench` uses).
