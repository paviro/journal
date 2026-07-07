use journal_storage::{Entry, JournalStore, Metadata};
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
    let recipient = store.initialize_encryption(passphrase).unwrap();
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
fn log_command_creates_entry_in_default_journal() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config)
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
        .arg(&config_path)
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
        .arg(&config)
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
        .arg(&config)
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
    store.unlock("secret").unwrap();
    assert!(
        store
            .read_entry_content(&encrypted_entry)
            .unwrap()
            .contains("# Secret")
    );
    assert_eq!(
        store
            .paths()
            .recipients_file
            .file_name()
            .and_then(|name| name.to_str()),
        Some(".recipients.txt")
    );
    assert_eq!(store.paths().recipients_file, root.join(".recipients.txt"));
    assert_eq!(store.paths().identity_file, dir.path().join("identity.age"));
    assert!(store.paths().recipients_file.exists());
    assert!(store.paths().identity_file.exists());
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
    store.unlock("secret").unwrap();
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
    store.unlock("secret").unwrap();
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
    let (store, _recipient) = generate_identity_store(&config, &root, "secret");
    let encrypted = store
        .create_entry_with_body("work", "# Secret", &Metadata::default())
        .unwrap();
    fs::remove_file(&store.paths().recipients_file).unwrap();
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
    assert!(!store.paths().recipients_file.exists());
}

#[test]
fn encrypted_entry_command_writes_age_files_without_unlocking() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("journals");
    let config = dir.path().join("config.toml");
    let (mut store, _recipient) = generate_identity_store(&config, &root, "secret");
    store.unlock("secret").unwrap();
    fs::remove_file(&store.paths().identity_file).unwrap();
    fs::create_dir_all(root.join("work")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
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
    store.unlock("secret").unwrap();
    fs::remove_file(&store.paths().identity_file).unwrap();
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
        .arg(&config)
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
    let (identity, recipient) = generate_age_cli_identity(dir.path());
    let store = JournalStore::for_config(&config, &root).unwrap();
    fs::create_dir_all(root.join("work")).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(&store.paths().recipients_file, format!("{recipient}\n")).unwrap();
    write_config(&config, &root, Some("work"));

    let output = Command::new(journal_bin())
        .arg("--config")
        .arg(&config)
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
