use std::{
    env, fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use tempfile::tempdir;

fn journal_bin() -> &'static str {
    env!("CARGO_BIN_EXE_journal")
}

fn write_config(path: &Path, root: &Path, default_journal: Option<&str>) {
    let mut config = journal::config::Config::new(root.to_path_buf(), "true");
    config.default_journal = default_journal.map(str::to_string);
    journal::config::save_config(path, &config).unwrap();
}

fn entry_texts(root: &Path, journal: &str) -> Vec<String> {
    let mut entries = journal::storage::scan_entries(root).unwrap();
    entries.retain(|entry| entry.journal == journal);
    entries
        .into_iter()
        .map(|entry| fs::read_to_string(entry.path).unwrap())
        .collect()
}

fn generate_identity_store(
    config: &Path,
    root: &Path,
    passphrase: &str,
) -> (journal::crypto::EncryptionPaths, String) {
    let paths = journal::crypto::EncryptionPaths::for_config(config, root).unwrap();
    let recipient = journal::crypto::generate_identity_store(&paths, passphrase).unwrap();
    (paths, recipient)
}

fn age_cli_available() -> bool {
    Command::new("age").arg("--version").output().is_ok()
        && Command::new("age-keygen").arg("--version").output().is_ok()
}

fn generate_age_cli_identity(dir: &Path) -> (std::path::PathBuf, String) {
    let identity = dir.join("age-identity.txt");
    let output = Command::new("age-keygen")
        .arg("-o")
        .arg(&identity)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let recipient = output_text
        .lines()
        .find_map(|line| line.strip_prefix("Public key: "))
        .expect("age-keygen output did not include public key")
        .to_string();

    (identity, recipient)
}

#[test]
fn positional_entry_command_creates_entry_in_default_journal() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = entry_texts(&root, "work");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("\n+++\n\nSome text\n"));
}

#[test]
fn entry_command_writes_tags() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--tag")
        .arg("rust")
        .arg("--tag")
        .arg("open source")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = entry_texts(&root, "work");
    assert_eq!(entries.len(), 1);
    let (front_matter, _) = journal::markdown::split_front_matter(&entries[0]);
    assert_eq!(
        front_matter.map(journal::markdown::front_matter_tags),
        Some(vec!["rust".to_string(), "open source".to_string()])
    );
}

#[test]
fn entry_command_accepts_comma_separated_tags() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--tag")
        .arg("rust,open source")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = entry_texts(&root, "work");
    let (front_matter, _) = journal::markdown::split_front_matter(&entries[0]);
    assert_eq!(
        front_matter.map(journal::markdown::front_matter_tags),
        Some(vec!["rust".to_string(), "open source".to_string()])
    );
}

#[test]
fn entry_command_accepts_comma_separated_feelings() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--feeling")
        .arg("calm,focused")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = entry_texts(&root, "work");
    let (front_matter, _) = journal::markdown::split_front_matter(&entries[0]);
    assert_eq!(
        front_matter.map(journal::markdown::front_matter_feelings),
        Some(vec!["calm".to_string(), "focused".to_string()])
    );
}

#[test]
fn entry_command_writes_repeatable_feelings() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--feeling")
        .arg("Calm")
        .arg("--feeling")
        .arg("focused")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = entry_texts(&root, "work");
    assert_eq!(entries.len(), 1);
    let (front_matter, _) = journal::markdown::split_front_matter(&entries[0]);
    assert_eq!(
        front_matter.map(journal::markdown::front_matter_feelings),
        Some(vec!["calm".to_string(), "focused".to_string()])
    );
}

#[test]
fn entry_command_rejects_unknown_feeling() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--feeling")
        .arg("sparkly")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown feeling 'sparkly'"));
    assert!(entry_texts(&root, "work").is_empty());
}

#[test]
fn piped_entry_command_creates_entry_in_default_journal() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let mut child = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"Line one\n\nLine three")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    let entries = entry_texts(&root, "work");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].ends_with("Line one\n\nLine three\n"));
}

#[test]
fn journal_flag_overrides_default_journal() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    fs::create_dir_all(root.join("personal")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--journal")
        .arg("personal")
        .arg("Override text")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(entry_texts(&root, "work").is_empty());
    let entries = entry_texts(&root, "personal");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("Override text\n"));
}

#[test]
fn set_default_journal_persists_to_config() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config_path = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config_path, &root, None);

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config_path)
        .arg("default")
        .arg("work")
        .output()
        .unwrap();

    assert!(output.status.success());
    let config = journal::config::load_config(&config_path).unwrap();
    assert_eq!(config.default_journal.as_deref(), Some("work"));
}

#[test]
fn entry_command_without_default_or_journal_fails() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, None);

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("Some text")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no journal specified"));
    assert!(entry_texts(&root, "work").is_empty());
}

#[test]
fn entry_command_rejects_text_and_piped_stdin_together() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let mut child = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("Arg text")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"Pipe text")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("piped stdin"));
    assert!(entry_texts(&root, "work").is_empty());
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

    let entry = journal::storage::create_entry(root.path(), "work", script.to_str().unwrap())
        .unwrap()
        .unwrap();
    let entry_text = fs::read_to_string(entry).unwrap();
    assert!(entry_text.contains("# Edited"));
}

#[test]
fn encrypt_command_converts_workspace_and_entry_command_writes_encrypted_files() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (paths, _recipient) = generate_identity_store(&config, &root, "secret");
    let entry_dir = root.join("work").join("2026").join("07").join("02");
    let trash_dir = root
        .join("work")
        .join(".trash")
        .join("2026")
        .join("07")
        .join("01");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::create_dir_all(&trash_dir).unwrap();
    let entry = entry_dir.join("entry.md");
    let trashed = trash_dir.join("old.md");
    fs::write(&entry, "+++\ntags = []\n+++\n\n# Secret\nBody\n").unwrap();
    fs::write(&trashed, "+++\ntags = []\n+++\n\n# Trashed\n").unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("encrypt")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let encrypted_entry = entry_dir.join("entry.md.age");
    let encrypted_trash = trash_dir.join("old.md.age");
    assert!(!entry.exists());
    assert!(encrypted_entry.exists());
    assert!(encrypted_trash.exists());
    let unlocked = journal::crypto::unlock_identity(&paths, "secret").unwrap();
    assert!(
        journal::crypto::decrypt_to_string(&unlocked, &encrypted_entry)
            .unwrap()
            .contains("# Secret")
    );
    assert_eq!(
        paths
            .recipients_file
            .file_name()
            .and_then(|name| name.to_str()),
        Some("recipients.txt")
    );
    assert_eq!(paths.recipients_file, root.join("recipients.txt"));
    assert_eq!(paths.identity_file, dir.path().join("identity.age"));
    assert!(paths.recipients_file.exists());
    assert!(paths.identity_file.exists());
    assert!(!dir.path().join("encryption").exists());
    assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("backup")
    }));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--journal")
        .arg("work")
        .arg("New encrypted body")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let created = Path::new(std::str::from_utf8(&output.stdout).unwrap().trim()).to_path_buf();
    assert!(
        created
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".md.age")
    );
    assert!(
        journal::crypto::decrypt_to_string(&unlocked, &created)
            .unwrap()
            .contains("New encrypted body")
    );
}

#[test]
fn encrypt_command_can_be_rerun_when_workspace_is_already_encrypted() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (paths, _recipient) = generate_identity_store(&config, &root, "secret");
    let encrypted =
        journal::storage::create_encrypted_entry_with_body(&root, "work", "# Secret\nBody", &paths)
            .unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("encrypt")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(encrypted.exists());
    let unlocked = journal::crypto::unlock_identity(&paths, "secret").unwrap();
    assert!(
        journal::crypto::decrypt_to_string(&unlocked, &encrypted)
            .unwrap()
            .contains("# Secret")
    );
    assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("backup")
    }));
}

#[test]
fn encrypt_command_finishes_partial_encryption_without_touching_existing_age_files() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (paths, _recipient) = generate_identity_store(&config, &root, "secret");
    let existing_encrypted =
        journal::storage::create_encrypted_entry_with_body(&root, "work", "# Existing", &paths)
            .unwrap();
    let entry_dir = root.join("work").join("2026").join("07").join("02");
    fs::create_dir_all(&entry_dir).unwrap();
    let remaining_plain = entry_dir.join("remaining.md");
    fs::write(&remaining_plain, "+++\ntags = []\n+++\n\n# Remaining\n").unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("encrypt")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let remaining_encrypted = entry_dir.join("remaining.md.age");
    assert!(existing_encrypted.exists());
    assert!(!remaining_plain.exists());
    assert!(remaining_encrypted.exists());
    let unlocked = journal::crypto::unlock_identity(&paths, "secret").unwrap();
    assert!(
        journal::crypto::decrypt_to_string(&unlocked, &existing_encrypted)
            .unwrap()
            .contains("# Existing")
    );
    assert!(
        journal::crypto::decrypt_to_string(&unlocked, &remaining_encrypted)
            .unwrap()
            .contains("# Remaining")
    );
}

#[test]
fn encrypt_command_fails_when_plain_entry_target_age_file_already_exists() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (paths, _recipient) = generate_identity_store(&config, &root, "secret");
    let entry_dir = root.join("work").join("2026").join("07").join("02");
    fs::create_dir_all(&entry_dir).unwrap();
    let plain = entry_dir.join("entry.md");
    let encrypted = entry_dir.join("entry.md.age");
    fs::write(&plain, "+++\ntags = []\n+++\n\n# Plain\n").unwrap();
    journal::crypto::encrypt_to_file(&paths, b"# Existing encrypted\n", &encrypted).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("encrypt")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("target already exists"));
    assert!(plain.exists());
    assert!(encrypted.exists());
}

#[test]
fn encrypt_command_fails_when_encrypted_entries_exist_without_recipients_file() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (paths, _recipient) = generate_identity_store(&config, &root, "secret");
    let encrypted =
        journal::storage::create_encrypted_entry_with_body(&root, "work", "# Secret", &paths)
            .unwrap();
    fs::remove_file(&paths.recipients_file).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("encrypt")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("recipients file is missing"));
    assert!(encrypted.exists());
    assert!(!paths.recipients_file.exists());
}

#[test]
fn encrypted_entry_command_writes_age_files_without_unlocking() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (paths, _recipient) = generate_identity_store(&config, &root, "secret");
    let unlocked = journal::crypto::unlock_identity(&paths, "secret").unwrap();
    fs::remove_file(&paths.identity_file).unwrap();
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--journal")
        .arg("work")
        .arg("age readable body")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let encrypted = Path::new(std::str::from_utf8(&output.stdout).unwrap().trim()).to_path_buf();
    assert!(
        encrypted
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".md.age")
    );
    let decrypted = journal::crypto::decrypt_to_string(&unlocked, &encrypted).unwrap();

    assert!(decrypted.contains("age readable body"));
}

#[test]
fn encrypted_entries_can_be_decrypted_with_age_cli() {
    if !age_cli_available() {
        return;
    }

    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (identity, recipient) = generate_age_cli_identity(dir.path());
    let paths = journal::crypto::EncryptionPaths::for_config(&config, &root).unwrap();
    fs::create_dir_all(root.join("work")).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(&paths.recipients_file, format!("{recipient}\n")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
        .arg("--journal")
        .arg("work")
        .arg("age CLI readable body")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let encrypted = Path::new(std::str::from_utf8(&output.stdout).unwrap().trim()).to_path_buf();

    let output = Command::new("age")
        .arg("--decrypt")
        .arg("--identity")
        .arg(&identity)
        .arg(&encrypted)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let decrypted = String::from_utf8(output.stdout).unwrap();

    assert!(decrypted.contains("age CLI readable body"));
}
