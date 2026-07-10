//! Builds the signed macOS location helper `.app` that `device_location/macos.rs`
//! embeds — a bare CLI can't get CoreLocation authorization on modern macOS, so
//! `journal-locate` is wrapped in a signed `.app`, zipped, and run at runtime.
//!
//! macOS targets only. Ad-hoc signed without credentials; Developer-ID signed and
//! (on release) notarized + stapled when `APPLE_*` are set.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let locate_crate = workspace.join("crates/journal-locate");
    let plist_src = locate_crate.join("JournalLocate-Info.plist");
    let entitlements = locate_crate.join("JournalLocate.entitlements");

    // Only the helper's own inputs should retrigger this script.
    for path in [
        locate_crate.join("src/main.rs"),
        locate_crate.join("Cargo.toml"),
        plist_src.clone(),
        entitlements.clone(),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for var in ["APPLE_DEVELOPER_ID", "APPLE_USERNAME", "APPLE_PASSWORD"] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let version = env::var("CARGO_PKG_VERSION").unwrap();

    let binary = build_helper(&target, &out_dir);
    let app = assemble_bundle(&binary, &plist_src, &out_dir, &version);
    sign(&app, &entitlements);
    if env::var("PROFILE").as_deref() == Ok("release") {
        notarize_and_staple(&app, &out_dir);
    }

    let zip = out_dir.join("JournalLocate.app.zip");
    ditto(&["-c", "-k", "--keepParent"], &app, &zip);
    println!(
        "cargo:rustc-env=JOURNAL_LOCATE_HELPER_ZIP={}",
        zip.display()
    );
}

/// Its own target dir keeps this nested cargo build clear of the outer build's lock.
fn build_helper(target: &str, out_dir: &Path) -> PathBuf {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target_dir = out_dir.join("helper-target");
    run(
        Command::new(cargo)
            .args(["build", "--release", "-p", "journal-locate", "--target"])
            .arg(target)
            .arg("--target-dir")
            .arg(&target_dir),
        "build the journal-locate helper",
    );
    target_dir.join(target).join("release/journal-locate")
}

/// The stamped version is what the runtime compares to decide an extracted copy
/// is current.
fn assemble_bundle(binary: &Path, plist_src: &Path, out_dir: &Path, version: &str) -> PathBuf {
    let app = out_dir.join("JournalLocate.app");
    let _ = fs::remove_dir_all(&app);
    let macos_dir = app.join("Contents/MacOS");
    fs::create_dir_all(&macos_dir).expect("create bundle MacOS dir");
    fs::copy(binary, macos_dir.join("journal-locate")).expect("copy helper binary");

    let plist = app.join("Contents/Info.plist");
    fs::copy(plist_src, &plist).expect("copy Info.plist");
    run(
        Command::new("/usr/bin/plutil")
            .args(["-replace", "CFBundleShortVersionString", "-string", version])
            .arg(&plist),
        "stamp the bundle version",
    );
    app
}

/// Ad-hoc signing still gets CoreLocation on the building machine, so a plain
/// `cargo build` needs no setup. Developer-ID signing is release-only; otherwise
/// a globally-set `APPLE_DEVELOPER_ID` would break `cargo check` on machines that
/// do not have the certificate installed.
fn sign(app: &Path, entitlements: &Path) {
    let mut cmd = Command::new("codesign");
    cmd.args(["--force", "--entitlements"]).arg(entitlements);
    let developer_id = (env::var("PROFILE").as_deref() == Ok("release"))
        .then(|| env_nonempty("APPLE_DEVELOPER_ID"))
        .flatten();
    match developer_id {
        Some(id) => {
            cmd.args(["--options", "runtime", "--timestamp", "--sign", &id]);
        }
        None => {
            cmd.args(["--sign", "-"]);
        }
    }
    run(cmd.arg(app), "sign the location helper");
}

/// Stapling embeds the ticket so the extracted `.app` validates offline. An ad-hoc
/// build (no `APPLE_DEVELOPER_ID`) has nothing to notarize; a Developer-ID build is
/// distributable, so notarization is required and any gap fails the build.
fn notarize_and_staple(app: &Path, out_dir: &Path) {
    let Some(developer_id) = env_nonempty("APPLE_DEVELOPER_ID") else {
        return;
    };
    let user = env_nonempty("APPLE_USERNAME")
        .expect("APPLE_DEVELOPER_ID is set but APPLE_USERNAME is missing — cannot notarize the location helper");
    let password = env_nonempty("APPLE_PASSWORD")
        .expect("APPLE_DEVELOPER_ID is set but APPLE_PASSWORD is missing — cannot notarize the location helper");
    let team_id = developer_id
        .rsplit_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(id, _)| id.to_string())
        .expect("APPLE_DEVELOPER_ID has no team id in parentheses — cannot notarize the location helper");

    let notary_zip = out_dir.join("JournalLocate-notary.zip");
    ditto(&["-c", "-k", "--keepParent"], app, &notary_zip);
    let mut submit = Command::new("xcrun");
    submit.args([
        "notarytool",
        "submit",
        notary_zip.to_str().unwrap(),
        "--apple-id",
        &user,
        "--password",
        &password,
        "--team-id",
        &team_id,
        "--wait",
    ]);
    // A network blip during the long `--wait` poll fails the whole command;
    // resubmitting the same zip is cheap and Apple accepts duplicates. Blips
    // last longer than an instant retry, so wait out the hiccup between tries.
    for attempt in 1..=4 {
        let status = submit
            .status()
            .unwrap_or_else(|error| panic!("could not notarize the location helper: {error}"));
        if status.success() {
            break;
        }
        assert!(
            attempt < 4,
            "failed to notarize the location helper (exit {status})"
        );
        println!("cargo:warning=notarization attempt {attempt} failed (exit {status}); retrying in 30s");
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    run(
        Command::new("xcrun").arg("stapler").arg("staple").arg(app),
        "staple the notarization ticket",
    );
}

fn ditto(flags: &[&str], src: &Path, dest: &Path) {
    let _ = fs::remove_file(dest);
    run(
        Command::new("/usr/bin/ditto")
            .args(flags)
            .arg(src)
            .arg(dest),
        "package the location helper with ditto",
    );
}

fn run(cmd: &mut Command, what: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|error| panic!("could not {what}: {error}"));
    assert!(status.success(), "failed to {what} (exit {status})");
}

fn env_nonempty(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.is_empty())
}
