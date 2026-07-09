//! Generates the third-party license report shown by `journal licenses`.
//!
//! Runs `cargo-about` over the dependency tree, groups crates by their license
//! text, and writes a gzipped JSON blob into `OUT_DIR` that the binary embeds
//! via `include_bytes!`. Set `JOURNAL_SKIP_LICENSE_GENERATION=1` to skip the
//! `cargo-about` call (writing an empty report) when the tool isn't installed —
//! the `journal licenses` command still prints the data-source attributions.

use flate2::{Compression, write::GzEncoder};
use serde::{Deserialize, Serialize};
use std::{env, fs, io::Write, path::Path, process::Command};

#[derive(Serialize)]
struct LicenseGroup {
    license: String,
    text: String,
    dependencies: Vec<Dependency>,
}

#[derive(Serialize)]
struct Dependency {
    name: String,
    version: String,
}

/// The slice of `cargo about generate --format json` we care about; every other
/// field in its (large) crate objects is ignored.
#[derive(Deserialize)]
struct AboutOutput {
    licenses: Vec<AboutLicense>,
}

#[derive(Deserialize)]
struct AboutLicense {
    id: String,
    text: String,
    used_by: Vec<AboutUsedBy>,
}

#[derive(Deserialize)]
struct AboutUsedBy {
    #[serde(rename = "crate")]
    krate: AboutCrate,
}

#[derive(Deserialize)]
struct AboutCrate {
    name: String,
    version: String,
}

fn main() {
    // Regenerate only when the dependency set or the allowlist changes.
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=about.toml");
    println!("cargo:rerun-if-env-changed=JOURNAL_SKIP_LICENSE_GENERATION");

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let output_path = Path::new(&out_dir).join("LICENSES.json.gz");

    if env::var_os("JOURNAL_SKIP_LICENSE_GENERATION").is_some() {
        println!(
            "cargo:warning=Skipping third-party license generation \
             (JOURNAL_SKIP_LICENSE_GENERATION set); `journal licenses` will list no dependencies."
        );
        write_gzipped(&output_path, b"[]");
        return;
    }

    let target = env::var("TARGET").expect("TARGET not set");
    let output = Command::new("cargo")
        .args(["about", "generate", "--format", "json", "--target", &target])
        .output();

    let json = match output {
        Ok(result) if result.status.success() => result.stdout,
        Ok(result) => panic!(
            "cargo-about failed to generate license data:\n{}",
            String::from_utf8_lossy(&result.stderr)
        ),
        Err(err) => panic!(
            "failed to run cargo-about (install it with `cargo install cargo-about`, \
             or set JOURNAL_SKIP_LICENSE_GENERATION=1 to skip): {err}"
        ),
    };

    let parsed: AboutOutput =
        serde_json::from_slice(&json).expect("failed to parse cargo-about JSON output");

    // Group crates by identical (license id, text) so a license block is stored
    // once regardless of how many crates share it.
    let mut groups: Vec<LicenseGroup> = Vec::new();
    for license in parsed.licenses {
        let deps = license.used_by.into_iter().map(|used| Dependency {
            name: used.krate.name,
            version: used.krate.version,
        });
        match groups
            .iter_mut()
            .find(|group| group.license == license.id && group.text == license.text)
        {
            Some(group) => group.dependencies.extend(deps),
            None => groups.push(LicenseGroup {
                license: license.id,
                text: license.text,
                dependencies: deps.collect(),
            }),
        }
    }
    for group in &mut groups {
        group
            .dependencies
            .sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
        group
            .dependencies
            .dedup_by(|a, b| a.name == b.name && a.version == b.version);
    }

    let serialized = serde_json::to_string(&groups).expect("failed to serialize license data");
    write_gzipped(&output_path, serialized.as_bytes());
}

fn write_gzipped(path: &Path, data: &[u8]) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data).expect("gzip write");
    let compressed = encoder.finish().expect("gzip finish");
    fs::write(path, compressed).expect("write LICENSES.json.gz");
}
