//! Filesystem locations shared across crates.

/// `~/Library/Application Support/de.paviro.notema` — the app's macOS support
/// directory, namespaced by the reverse-DNS bundle id. `None` when `HOME` is unset.
#[cfg(target_os = "macos")]
pub fn macos_support_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home).join("Library/Application Support/de.paviro.notema")
    })
}
