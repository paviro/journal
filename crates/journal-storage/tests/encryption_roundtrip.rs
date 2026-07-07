//! Encrypt → decrypt round-trip through the public store API. Encryption is
//! covered end-to-end by the CLI tests; this fills the decrypt gap: that
//! `decrypt_store` restores readable plaintext, drops the recipients file,
//! disables the identity, and leaves a backup.

use journal_storage::{JournalStore, Metadata, SecretString};

fn store_at(dir: &std::path::Path) -> JournalStore {
    JournalStore::new(dir.join("journals"), dir)
}

fn pw(passphrase: &str) -> SecretString {
    SecretString::from(passphrase)
}

#[test]
fn decrypt_store_restores_plaintext_and_disables_encryption() {
    let dir = tempfile::tempdir().unwrap();

    let mut store = store_at(dir.path());
    store.ensure().unwrap();
    store
        .initialize_encryption("laptop", Some(&pw("passphrase")))
        .unwrap();
    store.unlock(Some(&pw("passphrase"))).unwrap();
    assert!(store.encryption_enabled());

    store.create_journal("diary").unwrap();
    let path = store
        .create_entry_with_body("diary", "# Secret\nhidden body", &Metadata::default())
        .unwrap();
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("age"));

    let summary = store.decrypt_store(|_, _| {}).unwrap();
    assert!(summary.migrated_files >= 1);
    assert!(summary.backup_path.is_some_and(|p| p.exists()));
    assert!(summary.disabled_identity_file.exists());

    // Recipients gone → a fresh store treats everything as plaintext and reads it.
    assert!(
        !dir.path()
            .join("journals")
            .join(".age")
            .join("recipients.toml")
            .exists()
    );
    let plain = store_at(dir.path());
    assert!(!plain.encryption_enabled());
    let entries = plain.scan_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("hidden body"));
    assert!(entries[0].path.to_string_lossy().ends_with(".md"));
}

#[test]
fn decrypt_store_requires_an_unlocked_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_at(dir.path());
    store.ensure().unwrap();
    store
        .initialize_encryption("laptop", Some(&pw("passphrase")))
        .unwrap();

    // Never unlocked → decrypt refuses with the locked-identity error.
    let error = store.decrypt_store(|_, _| {}).unwrap_err();
    assert!(
        error
            .downcast_ref::<journal_storage::StorageError>()
            .is_some_and(|e| matches!(e, journal_storage::StorageError::LockedIdentity { .. }))
    );
}

#[test]
fn add_recipient_rolls_back_when_reencrypt_fails() {
    let dir = tempfile::tempdir().unwrap();
    let mut laptop = store_at(dir.path());
    laptop.ensure().unwrap();
    laptop
        .initialize_encryption("laptop", Some(&pw("pw")))
        .unwrap();
    laptop.unlock(Some(&pw("pw"))).unwrap();
    laptop.create_journal("diary").unwrap();
    let good = laptop
        .create_entry_with_body("diary", "keep me", &Metadata::default())
        .unwrap();
    let bad = laptop
        .create_entry_with_body("diary", "corrupt me", &Metadata::default())
        .unwrap();

    // A second device's recipient to add.
    let phone = JournalStore::new(dir.path().join("journals"), dir.path().join("phone"));
    phone.ensure().unwrap();
    let phone_recipient = phone.request_access("phone", None).unwrap();

    // Snapshot the good entry, then corrupt the other so re-encryption fails
    // partway through and the whole add must roll back.
    let good_before = std::fs::read(&good).unwrap();
    std::fs::write(&bad, b"not a valid age file").unwrap();

    let error = laptop
        .add_recipient(phone_recipient, |_, _| {})
        .unwrap_err();
    assert!(!error.to_string().is_empty());

    // Rolled back: still a single recipient, the good entry is byte-identical,
    // and no leftover backup directory.
    assert_eq!(laptop.recipients().unwrap().len(), 1);
    assert_eq!(std::fs::read(&good).unwrap(), good_before);
    let leftover_backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".backup-"));
    assert!(
        !leftover_backup,
        "backup dir should be cleaned up after rollback"
    );
}

#[test]
fn disable_clears_age_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let mut laptop = store_at(dir.path());
    laptop.ensure().unwrap();
    laptop
        .initialize_encryption("laptop", Some(&pw("pw")))
        .unwrap();
    laptop.unlock(Some(&pw("pw"))).unwrap();
    laptop.create_journal("diary").unwrap();
    laptop
        .create_entry_with_body("diary", "body", &Metadata::default())
        .unwrap();

    // A pending join request left sitting in the synced key folder.
    let phone = JournalStore::new(dir.path().join("journals"), dir.path().join("phone"));
    phone.ensure().unwrap();
    phone.request_access("phone", None).unwrap();
    let age_dir = dir.path().join("journals").join(".age");
    assert!(age_dir.join("recipients.toml").exists());
    assert!(!laptop.pending_requests().unwrap().is_empty());

    laptop.decrypt_store(|_, _| {}).unwrap();

    // Disabling encryption tears the whole key folder down, pending requests included.
    assert!(
        !age_dir.exists(),
        ".age folder should be gone after disable"
    );
}

#[test]
fn remove_recipient_refuses_own_device_even_after_rename() {
    let dir = tempfile::tempdir().unwrap();
    let mut laptop = store_at(dir.path());
    laptop.ensure().unwrap();
    laptop
        .initialize_encryption("laptop", Some(&pw("pw")))
        .unwrap();
    laptop.unlock(Some(&pw("pw"))).unwrap();

    // A second recipient so the "last recipient" guard isn't what stops us.
    let phone = JournalStore::new(dir.path().join("journals"), dir.path().join("phone"));
    phone.ensure().unwrap();
    let phone_recipient = phone.request_access("phone", None).unwrap();
    laptop.add_recipient(phone_recipient, |_, _| {}).unwrap();

    // Rename our own device — the local identity still stores the old name, so a
    // name-based guard would be bypassed here.
    laptop.rename_recipient("laptop", "renamed").unwrap();

    let error = laptop.remove_recipient("renamed", |_, _| {}).unwrap_err();
    assert!(error.to_string().contains("own recipient"));
    // Not locked out: still a current recipient.
    assert!(laptop.is_current_recipient().unwrap());
}

#[test]
fn approve_pending_is_idempotent_for_an_already_approved_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut laptop = store_at(dir.path());
    laptop.ensure().unwrap();
    laptop
        .initialize_encryption("laptop", Some(&pw("pw")))
        .unwrap();
    laptop.unlock(Some(&pw("pw"))).unwrap();
    laptop.create_journal("diary").unwrap();
    laptop
        .create_entry_with_body("diary", "history", &Metadata::default())
        .unwrap();

    let phone = JournalStore::new(dir.path().join("journals"), dir.path().join("phone"));
    phone.ensure().unwrap();
    phone.request_access("phone", None).unwrap();
    let request = laptop
        .pending_requests()
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    let first = laptop.approve_pending(&request, |_, _| {}).unwrap();
    assert!(first.migrated_files >= 1);
    assert_eq!(laptop.recipients().unwrap().len(), 2);

    // Approving the same request again (its key is now a recipient) is a no-op
    // that clears the stale request rather than erroring on the duplicate key.
    let second = laptop.approve_pending(&request, |_, _| {}).unwrap();
    assert_eq!(second.migrated_files, 0);
    assert_eq!(laptop.recipients().unwrap().len(), 2);
    assert!(laptop.pending_requests().unwrap().is_empty());
}

#[test]
fn non_recipient_device_reads_locked_placeholders_and_knows_it_is_pending() {
    let dir = tempfile::tempdir().unwrap();
    let mut laptop = store_at(dir.path());
    laptop.ensure().unwrap();
    laptop
        .initialize_encryption("laptop", Some(&pw("pw")))
        .unwrap();
    laptop.unlock(Some(&pw("pw"))).unwrap();
    laptop.create_journal("diary").unwrap();
    laptop
        .create_entry_with_body("diary", "secret", &Metadata::default())
        .unwrap();

    // A phone that enrolled but hasn't been approved: it has its own identity but
    // isn't a recipient.
    let mut phone = JournalStore::new(dir.path().join("journals"), dir.path().join("phone"));
    phone.ensure().unwrap();
    phone.request_access("phone", None).unwrap();
    phone.unlock(None).unwrap();

    assert!(!phone.is_current_recipient().unwrap());
    assert!(phone.self_request_pending().unwrap());

    // Scanning history it can't decrypt degrades to a locked placeholder instead
    // of failing the whole scan.
    let entries = phone.scan_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(matches!(
        entries[0].encryption_state,
        journal_storage::EntryEncryptionState::EncryptedLocked
    ));
}

#[test]
fn rotate_identity_replaces_the_key_and_keeps_reading() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = store_at(dir.path());
    store.ensure().unwrap();
    store
        .initialize_encryption("laptop", Some(&pw("pw")))
        .unwrap();
    store.unlock(Some(&pw("pw"))).unwrap();
    store.create_journal("diary").unwrap();
    let path = store
        .create_entry_with_body("diary", "before rotation", &Metadata::default())
        .unwrap();
    let old_key = store.public_recipient().unwrap();

    // Two passes re-encrypt the single entry, so at least two files migrate.
    let summary = store.rotate_identity(Some(&pw("pw")), |_, _| {}).unwrap();
    assert!(summary.migrated_files >= 2);

    // The device is now the sole recipient under a fresh key.
    let recipients = store.recipients().unwrap();
    assert_eq!(recipients.len(), 1);
    assert_ne!(recipients[0].key, old_key);

    // The store still reads the entry via the rotated key, and so does a fresh
    // store loading the newly-committed identity file.
    assert!(
        store
            .read_entry_content(&path)
            .unwrap()
            .contains("before rotation")
    );
    let mut fresh = store_at(dir.path());
    fresh.unlock(Some(&pw("pw"))).unwrap();
    assert!(
        fresh
            .read_entry_content(&path)
            .unwrap()
            .contains("before rotation")
    );
}
