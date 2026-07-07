use journal_storage::{Entry, JournalStore, Metadata, SecretString};
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
    write_config_with_editor(path, root, default_journal, "true");
}

fn write_config_with_editor(path: &Path, root: &Path, default_journal: Option<&str>, editor: &str) {
    let mut config = journal::config::Config::new(root.to_path_buf(), editor);
    config.default_journal = default_journal.map(str::to_string);
    journal::config::save_config(path, &config).unwrap();
}

fn scan_entries_for(root: &Path, journal: &str) -> Vec<Entry> {
    let store = JournalStore::for_config(&root.join("config.toml"), root).unwrap();
    let mut entries = store.scan_entries().unwrap();
    entries.retain(|entry| entry.journal == journal);
    entries
}

fn generate_identity_store(config: &Path, root: &Path, passphrase: &str) -> (JournalStore, String) {
    let store = JournalStore::for_config(config, root).unwrap();
    let recipient = store
        .initialize_encryption("laptop", Some(&SecretString::from(passphrase)))
        .unwrap();
    (store, recipient)
}

fn png_bytes() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0u8; 16]);
    bytes
}

fn age_cli_available() -> bool {
    Command::new("age").arg("--version").output().is_ok()
        && Command::new("age-keygen").arg("--version").output().is_ok()
}

/// Pull this device's age secret key out of its plaintext `identity.age` so the
/// standard `age` CLI can decrypt what the journal wrote. The key material is
/// bundled inside the file; the `AGE-SECRET-KEY-…` bech32 string is unambiguous
/// to slice out without depending on the internal serialization.
fn extract_age_secret(identity_text: &str) -> String {
    let start = identity_text
        .find("AGE-SECRET-KEY-")
        .expect("identity file has no age secret key");
    identity_text[start..]
        .chars()
        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '-')
        .collect()
}

#[test]
fn log_command_creates_entry_in_default_journal() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = scan_entries_for(&root, "work");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("Some text"));
}

#[test]
fn log_command_ingests_local_image_asset() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));
    let image = dir.path().join("photo.png");
    fs::write(&image, png_bytes()).unwrap();

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg(image.to_string_lossy().as_ref())
        .output()
        .unwrap();

    assert!(output.status.success());
    let created = Path::new(std::str::from_utf8(&output.stdout).unwrap().trim());
    let content = fs::read_to_string(created).unwrap();
    let stem = journal_storage::entry_id(created).unwrap();
    let assets = created.parent().unwrap().join(format!("{stem}.assets"));

    assert!(content.contains(&format!("![]({stem}.assets/")));
    assert_eq!(fs::read_dir(&assets).unwrap().count(), 1);
}

#[test]
fn log_command_writes_tags() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("--tag")
        .arg("rust")
        .arg("--tag")
        .arg("open source")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = scan_entries_for(&root, "work");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].metadata.tags,
        vec!["rust".to_string(), "open source".to_string()]
    );
}

#[test]
fn log_command_accepts_comma_separated_tags() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("--tag")
        .arg("rust,open source")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = scan_entries_for(&root, "work");
    assert_eq!(
        entries[0].metadata.tags,
        vec!["rust".to_string(), "open source".to_string()]
    );
}

#[test]
fn log_command_writes_people_and_activities() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("--person")
        .arg("alex,sam")
        .arg("--activity")
        .arg("programming")
        .arg("--activity")
        .arg("cycling")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = scan_entries_for(&root, "work");
    assert_eq!(
        entries[0].metadata.people,
        vec!["alex".to_string(), "sam".to_string()]
    );
    assert_eq!(
        entries[0].metadata.activities,
        vec!["programming".to_string(), "cycling".to_string()]
    );
}

#[test]
fn log_command_accepts_comma_separated_feelings() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("--feeling")
        .arg("calm,focused")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = scan_entries_for(&root, "work");
    assert_eq!(
        entries[0].metadata.feelings,
        vec!["calm".to_string(), "focused".to_string()]
    );
}

#[test]
fn log_command_writes_repeatable_feelings() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("--feeling")
        .arg("Calm")
        .arg("--feeling")
        .arg("focused")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(output.status.success());
    let entries = scan_entries_for(&root, "work");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].metadata.feelings,
        vec!["calm".to_string(), "focused".to_string()]
    );
}

#[test]
fn log_command_rejects_unknown_feeling() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("--feeling")
        .arg("sparkly")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown feeling 'sparkly'"));
    assert!(scan_entries_for(&root, "work").is_empty());
}

#[test]
fn piped_log_command_creates_entry_in_default_journal() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let mut child = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
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
    let entries = scan_entries_for(&root, "work");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("Line one\n\nLine three"));
}

#[test]
fn editor_log_command_creates_entry_in_default_journal() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();

    let script = dir.path().join("fake-editor.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '# Edited\\nBody from fake editor\\n' >> \"$1\"\n",
    )
    .unwrap();
    let chmod = Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    assert!(chmod.success());
    write_config_with_editor(&config, &root, Some("work"), script.to_str().unwrap());

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let entries = scan_entries_for(&root, "work");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("# Edited"));
}

#[test]
fn editor_log_command_creates_no_entry_when_body_is_empty() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());
    assert!(scan_entries_for(&root, "work").is_empty());
}

#[test]
fn bare_text_is_rejected() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("Some text")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(scan_entries_for(&root, "work").is_empty());
}

#[test]
fn bare_piped_stdin_requires_log_command() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let mut child = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("journal log"));
    assert!(scan_entries_for(&root, "work").is_empty());
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
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("--journal")
        .arg("personal")
        .arg("Override text")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(scan_entries_for(&root, "work").is_empty());
    let entries = scan_entries_for(&root, "personal");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("Override text"));
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
        .arg(config_path.parent().unwrap())
        .arg("use")
        .arg("work")
        .output()
        .unwrap();

    assert!(output.status.success());
    let config = journal::config::load_config(&config_path).unwrap();
    assert_eq!(config.default_journal.as_deref(), Some("work"));
}

#[test]
fn log_command_without_default_or_journal_fails() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, None);

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
        .arg("Some text")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no journal specified"));
    assert!(scan_entries_for(&root, "work").is_empty());
}

#[test]
fn log_command_rejects_text_and_piped_stdin_together() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let mut child = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
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
    assert!(scan_entries_for(&root, "work").is_empty());
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

    let store = JournalStore::for_config(&root.path().join("config.toml"), root.path()).unwrap();
    let entry = store
        .create_entry_via_editor("work", &Metadata::default(), |body| {
            journal::editor::edit_body(script.to_str().unwrap(), body)
        })
        .unwrap()
        .unwrap();
    let entry_text = fs::read_to_string(entry).unwrap();
    assert!(entry_text.contains("# Edited"));
}

#[test]
fn encrypt_command_converts_store_and_entry_command_writes_encrypted_files() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (mut store, _recipient) = generate_identity_store(&config, &root, "secret");
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
        .arg(config.parent().unwrap())
        .args(["encryption", "enable"])
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
    store.unlock(Some(&SecretString::from("secret"))).unwrap();
    assert!(
        store
            .read_entry_content(&encrypted_entry)
            .unwrap()
            .contains("# Secret")
    );
    assert_eq!(
        store
            .paths()
            .keys
            .devices_file
            .file_name()
            .and_then(|name| name.to_str()),
        Some("devices.toml")
    );
    assert_eq!(
        store.paths().keys.devices_file,
        root.join(".age").join("devices.toml")
    );
    assert_eq!(
        store.paths().keys.identity_file,
        dir.path().join("identity.age")
    );
    assert!(store.paths().keys.devices_file.exists());
    assert!(store.paths().keys.identity_file.exists());
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
        .arg(config.parent().unwrap())
        .arg("log")
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
        store
            .read_entry_content(&created)
            .unwrap()
            .contains("New encrypted body")
    );
}

#[test]
fn encrypt_command_can_be_rerun_when_store_is_already_encrypted() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (mut store, _recipient) = generate_identity_store(&config, &root, "secret");
    let encrypted = store
        .create_entry_with_body("work", "# Secret\nBody", &Metadata::default())
        .unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .args(["encryption", "enable"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(encrypted.exists());
    store.unlock(Some(&SecretString::from("secret"))).unwrap();
    assert!(
        store
            .read_entry_content(&encrypted)
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
    let (mut store, _recipient) = generate_identity_store(&config, &root, "secret");
    let existing_encrypted = store
        .create_entry_with_body("work", "# Existing", &Metadata::default())
        .unwrap();
    let entry_dir = root.join("work").join("2026").join("07").join("02");
    fs::create_dir_all(&entry_dir).unwrap();
    let remaining_plain = entry_dir.join("remaining.md");
    fs::write(&remaining_plain, "+++\ntags = []\n+++\n\n# Remaining\n").unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .args(["encryption", "enable"])
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
    store.unlock(Some(&SecretString::from("secret"))).unwrap();
    assert!(
        store
            .read_entry_content(&existing_encrypted)
            .unwrap()
            .contains("# Existing")
    );
    assert!(
        store
            .read_entry_content(&remaining_encrypted)
            .unwrap()
            .contains("# Remaining")
    );
}

#[test]
fn encrypt_command_fails_when_plain_entry_target_age_file_already_exists() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (_store, _recipient) = generate_identity_store(&config, &root, "secret");
    let entry_dir = root.join("work").join("2026").join("07").join("02");
    fs::create_dir_all(&entry_dir).unwrap();
    let plain = entry_dir.join("entry.md");
    let encrypted = entry_dir.join("entry.md.age");
    fs::write(&plain, "+++\ntags = []\n+++\n\n# Plain\n").unwrap();
    fs::write(&encrypted, "# Existing encrypted\n").unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .args(["encryption", "enable"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("target already exists"));
    assert!(plain.exists());
    assert!(encrypted.exists());
}

#[test]
fn encrypt_command_fails_when_encrypted_entries_exist_without_device_roster() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (store, _recipient) = generate_identity_store(&config, &root, "secret");
    let encrypted = store
        .create_entry_with_body("work", "# Secret", &Metadata::default())
        .unwrap();
    fs::remove_file(&store.paths().keys.devices_file).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .args(["encryption", "enable"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("device roster is missing"));
    assert!(encrypted.exists());
    assert!(!store.paths().keys.devices_file.exists());
}

#[test]
fn encrypt_command_fails_when_recipients_exist_but_device_has_no_identity() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    // Recipients synced from another device, but this one never enrolled.
    let (store, _recipient) = generate_identity_store(&config, &root, "secret");
    fs::remove_file(&store.paths().keys.identity_file).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .args(["encryption", "enable"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("device enroll"));
}

#[test]
fn encrypted_entry_command_writes_age_files_without_unlocking() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (mut store, _recipient) = generate_identity_store(&config, &root, "secret");
    store.unlock(Some(&SecretString::from("secret"))).unwrap();
    fs::remove_file(&store.paths().keys.identity_file).unwrap();
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
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
    let decrypted = store.read_entry_content(&encrypted).unwrap();

    assert!(decrypted.contains("age readable body"));
}

#[test]
fn encrypted_editor_log_command_writes_age_files_without_unlocking() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (mut store, _recipient) = generate_identity_store(&config, &root, "secret");
    store.unlock(Some(&SecretString::from("secret"))).unwrap();
    fs::remove_file(&store.paths().keys.identity_file).unwrap();
    fs::create_dir_all(root.join("work")).unwrap();

    let script = dir.path().join("fake-editor.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '# Encrypted editor body\\n' >> \"$1\"\n",
    )
    .unwrap();
    let chmod = Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    assert!(chmod.success());
    write_config_with_editor(&config, &root, Some("work"), script.to_str().unwrap());

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
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
    let decrypted = store.read_entry_content(&encrypted).unwrap();

    assert!(decrypted.contains("# Encrypted editor body"));
}

#[test]
fn encrypted_entries_can_be_decrypted_with_age_cli() {
    if !age_cli_available() {
        return;
    }

    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    // A normal no-passphrase device: its own age key is the sole recipient, so the
    // standard age CLI can decrypt what the journal writes to prove it is real age.
    let store = JournalStore::for_config(&config, &root).unwrap();
    store.initialize_encryption("laptop", None).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .arg("log")
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

    let identity = dir.path().join("age-identity.txt");
    let secret =
        extract_age_secret(&fs::read_to_string(&store.paths().keys.identity_file).unwrap());
    fs::write(&identity, format!("{secret}\n")).unwrap();

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

/// Run the journal binary against `config` and assert success, returning stdout.
fn run_ok(config: &Path, args: &[&str]) -> String {
    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(config.parent().unwrap())
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn key_workflow_grants_second_device_history_access() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    fs::create_dir_all(root.join("work")).unwrap();
    // Two config dirs sharing one journal root simulate two devices; each keeps
    // its own identity next to its config.
    let laptop_cfg = dir.path().join("laptop/config.toml");
    let phone_cfg = dir.path().join("phone/config.toml");
    fs::create_dir_all(laptop_cfg.parent().unwrap()).unwrap();
    fs::create_dir_all(phone_cfg.parent().unwrap()).unwrap();
    write_config(&laptop_cfg, &root, Some("work"));
    write_config(&phone_cfg, &root, Some("work"));

    // Laptop enables encryption and writes an entry before the phone exists.
    run_ok(
        &laptop_cfg,
        &[
            "encryption",
            "enable",
            "--name",
            "laptop",
            "--no-passphrase",
        ],
    );
    run_ok(&laptop_cfg, &["log", "--journal", "work", "secret history"]);

    // Phone requests access; laptop lists it pending, then approves it.
    run_ok(
        &phone_cfg,
        &[
            "encryption",
            "device",
            "enroll",
            "--name",
            "phone",
            "--no-passphrase",
        ],
    );
    let listing = run_ok(&laptop_cfg, &["encryption", "device", "list"]);
    assert!(listing.contains("Pending approval"), "{listing}");
    assert!(listing.contains("phone"), "{listing}");
    run_ok(&laptop_cfg, &["encryption", "device", "approve", "--all"]);

    // After re-encryption the phone can read the entry written before it joined.
    let mut phone = JournalStore::for_config(&phone_cfg, &root).unwrap();
    phone.unlock(None).unwrap();
    let entries = phone.scan_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("secret history"));

    // Both devices are recipients; the pending request is cleared.
    assert_eq!(phone.recipients().unwrap().len(), 2);
    assert!(phone.pending_requests().unwrap().is_empty());
}

#[test]
fn encrypt_decrypt_converts_assets_and_keeps_clean_links() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));
    let image = dir.path().join("photo.png");
    fs::write(&image, png_bytes()).unwrap();

    // A plaintext entry with an ingested image; its body link is clean.
    run_ok(
        &config,
        &["log", "--journal", "work", image.to_string_lossy().as_ref()],
    );
    let plain = JournalStore::for_config(&config, &root).unwrap();
    let body = plain.scan_entries().unwrap().remove(0).content;
    assert!(
        body.contains(".assets/") && !body.contains(".age"),
        "{body}"
    );

    // Encrypt without a passphrase.
    run_ok(&config, &["encryption", "enable", "--no-passphrase"]);
    let mut enc = JournalStore::for_config(&config, &root).unwrap();
    assert!(!enc.identity_needs_passphrase().unwrap());
    enc.unlock(None).unwrap();
    let entry = enc.scan_entries().unwrap().remove(0);
    // The body is byte-for-byte unchanged (link still clean) though it's encrypted.
    assert_eq!(entry.content, body);
    assert!(entry.path.to_string_lossy().ends_with(".md.age"));

    // The asset on disk is now `.age`, and the clean link still resolves+decrypts.
    let stem = journal_storage::entry_id(&entry.path).unwrap();
    let assets_dir = entry.path.parent().unwrap().join(format!("{stem}.assets"));
    let asset = fs::read_dir(&assets_dir).unwrap().next().unwrap().unwrap();
    let asset_name = asset.file_name().into_string().unwrap();
    assert!(
        asset_name.ends_with(".age"),
        "asset encrypted: {asset_name}"
    );
    let clean = asset_name.strip_suffix(".age").unwrap();
    assert!(
        enc.read_entry_asset_bytes(&entry.path, clean)
            .unwrap()
            .is_some()
    );

    // Decrypt (plaintext identity → no unlock prompt): asset returns to plaintext, body unchanged.
    run_ok(&config, &["encryption", "disable", "--yes"]);
    let dec = JournalStore::for_config(&config, &root).unwrap();
    let dec_entry = dec.scan_entries().unwrap().remove(0);
    assert_eq!(dec_entry.content, body);
    let dec_stem = journal_storage::entry_id(&dec_entry.path).unwrap();
    let dec_assets = dec_entry
        .path
        .parent()
        .unwrap()
        .join(format!("{dec_stem}.assets"));
    let dec_asset = fs::read_dir(&dec_assets).unwrap().next().unwrap().unwrap();
    assert!(
        !dec_asset.file_name().to_string_lossy().ends_with(".age"),
        "asset plaintext again"
    );
}
