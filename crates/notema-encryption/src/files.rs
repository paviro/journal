use crate::Result;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// A unique hidden sibling temp path next to `target`, for atomic
/// write-then-rename. Named `.notema-<pid>-<rand>.<suffix>` in the target's
/// directory so it lands on the same filesystem as the eventual rename target.
pub fn sibling_temp_path(target: &Path, suffix: &str) -> Result<PathBuf> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let mut noise = [0u8; 8];
    getrandom::getrandom(&mut noise)
        .map_err(|error| crate::EncryptionError::Randomness(error.to_string()))?;
    Ok(parent.join(format!(
        ".notema-{}-{}.{suffix}",
        std::process::id(),
        hex::encode(noise),
    )))
}

/// Write `content` to `path` via a sibling temp file plus rename, so a crash
/// mid-write can't truncate an existing file (which would strand every device)
/// or leave a half-written join request behind.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    write_atomic(path, content, false)
}

/// Atomically write a file readable only by its owner (mode 0600 on Unix),
/// creating parent directories as needed.
pub fn atomic_write_private(path: &Path, content: &[u8]) -> Result<()> {
    write_atomic(path, content, true)
}

fn write_atomic(path: &Path, content: &[u8], private: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = sibling_temp_path(path, "tmp")?;
    let result = write_temp_then_rename(&temp, path, content, private);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn write_temp_then_rename(temp: &Path, path: &Path, content: &[u8], private: bool) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if private {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(temp)?;
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)?;
    sync_parent_dir(path);
    Ok(())
}

fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}
