//! macOS provider. A bare command-line binary can't obtain CoreLocation
//! authorization on modern macOS (Ventura+) — the request is denied with no
//! prompt. So the actual CoreLocation code lives in the tiny `notema-locate`
//! helper, wrapped in a **signed `.app`** (which *can* get location). That signed
//! `.app` is zipped at build time and embedded here; at runtime we extract it to
//! a stable per-build path and run it, reading the JSON fix it prints.
//!
//! The signature travels inside the files, so the extracted copy stays valid and
//! its location grant persists (locationd keys on the path + code hash).

use super::{DeviceFix, DeviceLocationSource, parse_fix_json};
use crate::AppResult;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// Matched against the extracted bundle's `CFBundleShortVersionString` (stamped
/// from this same version by `build.rs`) to decide whether to re-extract.
const EXPECTED_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The signed `NotemaLocate.app`, zipped (built by this crate's `build.rs`).
/// Extracted and run at runtime.
static HELPER_ZIP: &[u8] = include_bytes!(env!("JOURNAL_LOCATE_HELPER_ZIP"));

/// Outer guard around the helper process. The helper has its own auth/fix
/// timeouts; this only catches a wedged child.
const TIMEOUT: Duration = Duration::from_secs(90);

pub(super) fn locate() -> AppResult<DeviceFix> {
    let binary = ensure_helper()?;

    let mut child = Command::new(&binary)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| anyhow::anyhow!("could not run the location helper: {error}"))?;

    let mut stdout = child.stdout.take().expect("stdout piped above");
    let mut stderr = child.stderr.take().expect("stderr piped above");
    let output = super::run_with_timeout(TIMEOUT, move || {
        let mut out = String::new();
        let mut err = String::new();
        let _ = stdout.read_to_string(&mut out);
        let _ = stderr.read_to_string(&mut err);
        (out, err)
    });

    match output {
        Some((out, err)) => {
            let status = child.wait().ok();
            if status.is_some_and(|s| s.success()) {
                parse_fix_json(&out, DeviceLocationSource::CoreLocation)
            } else {
                // The helper prints a helpful reason (denied, timed out) on stderr.
                let message = err.trim();
                if message.is_empty() {
                    anyhow::bail!("the location helper failed to get a fix")
                } else {
                    anyhow::bail!("{message}")
                }
            }
        }
        None => {
            let _ = child.kill();
            anyhow::bail!("timed out waiting for the location helper")
        }
    }
}

/// Extract the embedded helper `.app` to a fixed path and return its executable,
/// re-extracting only when the on-disk bundle's version differs from this build's
/// (the version lives inside the bundle's `Info.plist`, so the support directory
/// holds nothing but the `.app`). On an update the `.app` is replaced in place;
/// its new code hash makes macOS re-prompt once, as it should for changed code.
fn ensure_helper() -> AppResult<PathBuf> {
    let dir = support_dir()?;
    let app = dir.join("NotemaLocate.app");
    let binary = app.join("Contents/MacOS/notema-locate");
    let plist = app.join("Contents/Info.plist");

    // Reuse the existing extraction when its bundled version matches this build.
    if binary.exists() && bundle_version(&plist).as_deref() == Some(EXPECTED_VERSION) {
        return Ok(binary);
    }

    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::remove_dir_all(&app);
    // Write the zip beside the target and extract with `ditto`, which preserves
    // the bundle layout and the code signature (a plain unzip can mangle it).
    let zip = dir.join("NotemaLocate.app.zip");
    std::fs::write(&zip, HELPER_ZIP)?;
    let status = Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(&zip)
        .arg(&dir)
        .status()
        .map_err(|error| anyhow::anyhow!("could not run ditto to unpack the helper: {error}"))?;
    let _ = std::fs::remove_file(&zip);
    if !status.success() {
        anyhow::bail!("failed to unpack the location helper");
    }
    if !binary.exists() {
        anyhow::bail!("location helper missing after unpack");
    }
    Ok(binary)
}

fn support_dir() -> AppResult<PathBuf> {
    notema_core::paths::macos_support_dir().ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

/// Read `CFBundleShortVersionString` from an extracted bundle's `Info.plist`.
/// `None` when the bundle is missing or unreadable, which forces a re-extract.
fn bundle_version(plist: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
