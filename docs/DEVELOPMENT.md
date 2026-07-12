# Development

## Setup

Needs the toolchain pinned in `rust-toolchain.toml` (Rust 1.96, with `clippy`
and `rustfmt`). `rustup` installs it automatically on first `cargo` invocation
in the repo.

## Checks

These mirror the CI jobs; run them before pushing.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

CI runs `cargo audit --deny warnings` as well; local runs need
`cargo install cargo-audit`. Acknowledged advisories are listed in
[AUDIT.md](AUDIT.md).

The `fuse` feature isn't in `--all-features` reach on every platform. To check
the mount wiring compiles:

```bash
cargo check -p notema --features fuse --locked   # needs libfuse3 headers
```

## Cross-compiled release builds

`Makefile.toml` drives the release matrix through
[`cargo-make`](https://github.com/sagiegurari/cargo-make):

```bash
cargo make build-termux            # Android/Termux ARM64
cargo make build-x86-gnu           # x86_64 Linux (glibc)
cargo make build-macos-universal   # Intel + Apple Silicon
cargo make build-windows-gnu       # Windows x86_64
cargo make run-tests               # full workspace test suite
```

See [BUILDING.md](BUILDING.md) for prerequisites and the FUSE variants.

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
