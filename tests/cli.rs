use std::{env, fs, process::Command};
use tempfile::tempdir;

fn journal_bin() -> &'static str {
    env!("CARGO_BIN_EXE_journal")
}

#[test]
fn positional_entry_command_is_rejected() {
    let output = Command::new(journal_bin())
        .arg("some entry")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
}

#[test]
fn fake_editor_command_edits_entry_files_in_place() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("work")).unwrap();

    let script = root.path().join("fake-editor.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '\\n# Edited\\nBody from fake editor\\n' >> \"$1\"\n",
    )
    .unwrap();
    let chmod = Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    assert!(chmod.success());

    let entry =
        journal::storage::create_entry(root.path(), "work", script.to_str().unwrap()).unwrap();
    let entry_text = fs::read_to_string(entry).unwrap();
    assert!(entry_text.contains("# Edited"));
}
