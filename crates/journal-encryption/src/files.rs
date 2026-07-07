use crate::Result;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// A unique hidden sibling temp path next to `target`, for atomic
/// write-then-rename. Named `.journal-<pid>-<rand>.<suffix>` in the target's
/// directory so it lands on the same filesystem as the eventual rename target.
pub fn sibling_temp_path(target: &Path, suffix: &str) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let mut noise = [0u8; 8];
    let _ = getrandom::getrandom(&mut noise);
    parent.join(format!(
        ".journal-{}-{}.{suffix}",
        std::process::id(),
        hex::encode(noise),
    ))
}

/// Write `content` to `path` via a sibling temp file plus rename, so a crash
/// mid-write can't truncate an existing file (which would strand every device)
/// or leave a half-written join request behind.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let temp = sibling_temp_path(path, "tmp");
    fs::write(&temp, content)?;
    fs::rename(&temp, path)?;
    Ok(())
}

/// Write a file readable only by its owner (mode 0600 on Unix), creating parent
/// directories as needed. Used for this device's private identity file.
pub(crate) fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(content)?;
    Ok(())
}
