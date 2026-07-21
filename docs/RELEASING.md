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
(`Actions → Release → Run workflow`) on a branch: it builds and uploads every
artifact as a workflow artifact but skips the publish step (guarded on a tag
push).

## What gets built, and on which runner

| Runner | Artifacts |
| --- | --- |
| `ubuntu-latest` | all glibc Linux targets (x86_64/aarch64/i686/armv7), x86_64/i686/armv7 musl, i586, Android/Termux, `linux-gnu-x86_64-fuse` |
| `ubuntu-24.04-arm` | aarch64 musl Linux and `linux-gnu-aarch64-fuse`, built natively |
| `windows-latest` | `windows-msvc-x86_64`, built natively |
| `windows-11-arm` | `windows-msvc-aarch64`, built natively |
| `macos-latest` | `apple-darwin-{x86_64,aarch64,universal}` and their `-fuse` variants (signed + notarized) |

The glibc targets are cross-compiled with [`cargo-zigbuild`], which links with
zig and pins the glibc floor at **2.17** regardless of the runner's own glibc,
so the binaries run on old distros (RHEL 7 / Ubuntu 14.04 era and later). The
musl targets are statically linked and cross-compiled via
[`taiki-e/setup-cross-toolchain-action`]. i586 (SSE-free, static OpenSSL) and
macOS reuse the `Makefile.toml` tasks, which can also be run locally (see
[`docs/BUILDING.md`](BUILDING.md)).

[`cargo-zigbuild`]: https://github.com/rust-cross/cargo-zigbuild
[`taiki-e/setup-cross-toolchain-action`]: https://github.com/taiki-e/setup-cross-toolchain-action

## Required secrets and the `release` environment

The macOS job signs with a Developer ID certificate and notarizes with an Apple
ID. To keep those credentials off arbitrary `workflow_dispatch` runs, the `macos`
and `publish` jobs are bound to a protected **`release` environment** (Settings
→ Environments → `release`) with a required reviewer, and the five secrets below
are stored **on that environment** rather than repo-wide. Every release — and
any rehearsal that reaches the macOS or publish job — therefore waits for
manual approval. The environment has no deployment-branch/tag policy, so
rehearsal runs from branches can still reach it; the reviewer sees the ref
before approving.

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

The macOS build also signs and notarizes the embedded location helper *during*
`cargo build` (see `crates/notema-context/build.rs`), so each macOS artifact is
notarized twice (helper, then outer zip) — expect the macOS job to take a while.

## Verifying a release

- Every expected zip plus `SHA256SUMS` is attached to the release
  (20 zips: 8 Linux + 2 Linux FUSE + i586 + Android + 2 Windows + 3 macOS +
  3 macOS FUSE).
- On macOS, `codesign --verify --strict` and `spctl -a -vv` pass for a downloaded
  binary; the notarized zips staple/validate.
- `file notema` inside each Linux/Android zip reports the expected architecture.
- The glibc floor held: for each `linux-gnu-*` binary,
  `strings notema | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1` prints
  `GLIBC_2.17` or lower.
- Each published zip carries a build-provenance attestation:
  `gh attestation verify <zip> --repo <owner>/<repo>` confirms it was produced by
  this workflow.
