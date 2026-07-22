# Releasing

Official releases are built and published by GitHub Actions
([`.github/workflows/release.yml`](../.github/workflows/release.yml)). Pushing a
version tag builds every shipped artifact, signs + notarizes the macOS builds,
generates checksums, and attaches everything to the GitHub Release for that tag.

## Cutting a release

1. Bump `version` in the root `Cargo.toml` (`workspace.package.version`) and commit.
2. Tag the commit with the same version and push the tag:

   ```bash
   git tag 2026.7.4
   git push origin 2026.7.4
   ```

   Versions are date-based (`YEAR.MONTH.PATCH`, e.g. `2026.7.4`). The tag must be
   three dot-separated numbers or the workflow won't trigger, and a preflight
   job fails the run immediately if the tag doesn't equal
   `workspace.package.version` in `Cargo.toml`.
3. The workflow builds all targets, publishes the release, and uploads the zips
   plus `SHA256SUMS`.

To rehearse without publishing, run the workflow manually
(`Actions → Release → Run workflow`) and choose a target group. `all` builds
the complete release; the other choices build only Linux, Linux FUSE, i586,
Android, FreeBSD, Windows, macOS, or macOS FUSE. Manual runs never publish, even
when run against a tag. Their workflow artifacts expire after one day.

## What gets built, and where

| Runner / container | Artifacts |
| --- | --- |
| `ubuntu-latest` | x86_64/i686/armv6/armv7/riscv64 glibc Linux, x86_64/i686/armv6/armv7/riscv64 musl, i586, Android/Termux, and both FreeBSD slices (zigbuild; smoke-tested in a FreeBSD 14.3 VM on the runner) |
| `ubuntu-24.04-arm` | aarch64 glibc (zigbuild, for the 2.17 floor) and aarch64 musl Linux |
| `almalinux:8.10` on matching x86_64/ARM64 runners | `linux-gnu-{x86_64,aarch64}-fuse`, built natively |
| `windows-latest` | `windows-msvc-x86_64`, built natively |
| `windows-11-arm` | `windows-msvc-aarch64`, built natively |
| `macos-latest` | Intel and Apple Silicon macOS, standard and FUSE, one job per artifact (signed + notarized) |

The standard glibc targets are cross-compiled with [`cargo-zigbuild`], which
links with zig and pins the glibc floor at **2.17** regardless of the runner's
own glibc — except riscv64, whose floor is **2.27**, the first glibc with that
port. The FUSE variants are built in AlmaLinux 8 and require **glibc 2.28**
plus `libfuse3.so.3`; older systems can use the standard artifacts without the
`mount` command. The musl targets are statically linked and cross-compiled via
[`taiki-e/setup-cross-toolchain-action`]. i586 (SSE-free, static OpenSSL) and
macOS reuse the `Makefile.toml` tasks, which can also be run locally (see
[`docs/BUILDING.md`](BUILDING.md)).

The FreeBSD artifacts are also cross-compiled with [`cargo-zigbuild`] — zig
ships the FreeBSD 14+ libc stubs and headers, so no BSD host is involved —
giving them a **FreeBSD 14** floor. `aarch64-unknown-freebsd` is a Tier 3 Rust
target with no prebuilt std, so that slice builds std from source
(`-Z build-std`) on a nightly toolchain date-pinned in the workflow and in
`Makefile.toml` (`FREEBSD_NIGHTLY`); bumping it is a deliberate act like the
stable-toolchain pin. On aarch64 FreeBSD, `ring` has no runtime CPU-feature
detection and falls back to baseline software crypto — slower TLS, functionally
identical, and the right default since common hardware there (Raspberry Pi 4)
lacks the ARMv8 crypto extensions.

The workflow rejects standard GNU binaries with symbols newer than their glibc
floor (2.17, or 2.27 for riscv64), musl binaries with a dynamic interpreter or
shared-library dependency, an Android artifact that is not ARM64 API 24, and
FreeBSD binaries without FreeBSD ELF branding (dynamic-linker path and
`.note.tag` note) on the expected architecture. The
macOS builds — standard and FUSE — pin and verify 10.12 for Intel and 11 for
Apple Silicon on both `notema` and the embedded location helper. Every artifact
except Android also gets a `--version` smoke test (with `LD_BIND_NOW=1` on
dynamically linked Linux builds, so the whole symbol chain must resolve):
natively where the runner can load the binary — including the Intel macOS
slices under Rosetta and 32-bit x86 on the x86_64 runners — under qemu-user for
armv6, armv7, and riscv64, which no hosted runner can run natively, and inside
a FreeBSD **14.3** VM booted on the runner for the FreeBSD slices, so a passing
run also proves their release floor. The static armv6 musl slice runs with
`-cpu arm1176` (the Pi Zero core), so ARMv7 instruction leakage fails the
test; the dynamically linked slices use qemu's default CPU because the Debian
armhf cross sysroot's loader is ARMv7. Android needs an emulator, so it stops
at the static checks.

[`cargo-zigbuild`]: https://github.com/rust-cross/cargo-zigbuild
[`taiki-e/setup-cross-toolchain-action`]: https://github.com/taiki-e/setup-cross-toolchain-action

## Required secrets and the `release` environment

The four macOS jobs sign with a Developer ID certificate and notarize with an
Apple ID. To keep those credentials off arbitrary `workflow_dispatch` runs, the
`macos`, `macos-fuse`, and `publish` jobs are bound to a protected **`release`
environment** (Settings → Environments → `release`) with a required reviewer,
and the five secrets below are stored **on that environment** rather than
repo-wide. Every release and any rehearsal that includes macOS therefore waits
for manual approval. The environment has no
deployment-branch/tag policy, so rehearsal runs from branches can still reach
it; the reviewer sees the ref before approving.

A tag **ruleset** additionally restricts creating, moving, or deleting version
tags (`*.*.*`) to repository admins, so only a maintainer can trigger a
release even if another account gains write access.

| Secret | What it is |
| --- | --- |
| `APPLE_DEVELOPER_ID` | The identity string, e.g. `Developer ID Application: Your Name (TEAMID)`. Find it with `security find-identity -v -p codesigning`. |
| `APPLE_USERNAME` | Apple ID email used for notarization. |
| `APPLE_PASSWORD` | An app-specific password for that Apple ID. |
| `APPLE_CERT_P12_BASE64` | The Developer ID Application certificate exported as a `.p12`, base64-encoded: `base64 -i cert.p12 \| pbcopy`. |
| `APPLE_CERT_PASSWORD` | The password set when exporting the `.p12`. |

`APPLE_DEVELOPER_ID` mirrors the local `./.env` (see
[`.env.example`](../.env.example)); the workflow writes it back into a temporary
`./.env` so the Makefile's file-gated sign/notarize tasks run. `APPLE_USERNAME`
and `APPLE_PASSWORD` never touch `.env`: the workflow stores them straight into
a **notarytool keychain profile** on the runner, and every `notarytool submit`
(Makefile tasks and `notema-context`'s build.rs) runs off the profile named by
`APPLE_NOTARY_PROFILE` in `.env`. For local releases the profile is a one-time
setup instead — pick any name (an existing profile shared with other projects
works too), and the password is prompted for, so the secret lives only in the
keychain:

```bash
xcrun notarytool store-credentials <name> --apple-id <apple-id> --team-id <team-id>
```

The last two secrets import the certificate into a throwaway keychain on the
runner so `codesign` can find the identity.

The macOS builds also sign and notarize the embedded location helper *during*
`cargo build` (see `crates/notema-context/build.rs`), so each macOS artifact is
notarized twice (helper, then outer zip). The four jobs run in parallel.

## Verifying a release

- Every expected zip plus `SHA256SUMS` is attached to the release
  (24 zips: 12 Linux + 2 Linux FUSE + i586 + Android + 2 FreeBSD + 2 Windows +
  2 macOS + 2 macOS FUSE).
- On macOS, `codesign --verify --strict` and `spctl -a -vv` pass for a downloaded
  binary; the notarized zips staple/validate.
- `file notema` inside each Linux/Android zip reports the expected architecture.
- The standard glibc floor held: for each non-FUSE `linux-gnu-*` binary,
  `strings notema | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1` prints
  `GLIBC_2.17` or lower (`GLIBC_2.27` for `linux-gnu-riscv64`).
- Every non-FUSE musl binary has no ELF interpreter or `DT_NEEDED` entry.
- `android-aarch64-termux.zip` is an AArch64 ELF declaring Android API 24.
- The two `freebsd-*` binaries are `FreeBSD-style` ELFs per `file`, request
  `/libexec/ld-elf.so.1`, and match their expected architectures.
- The macOS binaries (standard and FUSE) and embedded location helpers declare
  macOS 10.12 for Intel and macOS 11 for Apple Silicon.
- The two `linux-gnu-*-fuse` binaries require `libfuse3.so.3`, and the same
  command prints `GLIBC_2.28` or lower.
- Each published zip carries a build-provenance attestation:
  `gh attestation verify <zip> --repo <owner>/<repo>` confirms it was produced by
  this workflow.
