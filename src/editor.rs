use crate::AppResult;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn open_editor(editor: &str, path: &Path) -> AppResult<()> {
    let mut parts = shell_words::split(editor)?;
    if parts.is_empty() {
        return Err("editor command is empty".into());
    }

    let program = parts.remove(0);
    let status = Command::new(program).args(parts).arg(path).status()?;
    if !status.success() {
        return Err(format!("editor exited with status {status}").into());
    }
    Ok(())
}

/// Write `body` to a temporary file, open the editor on it, and return the
/// edited text. Returns `None` if the editor exits with an error.
pub fn edit_body(editor: &str, body: &str) -> AppResult<Option<String>> {
    let temp = unique_temp_path("body.md");
    fs::write(&temp, body)?;
    let result = open_editor(editor, &temp);
    let new_body = if result.is_ok() {
        Some(fs::read_to_string(&temp)?)
    } else {
        None
    };
    let _ = fs::remove_file(&temp);
    result?;
    Ok(new_body)
}

fn unique_temp_path(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(".journal-{}-{nanos}.{suffix}", std::process::id()))
}
