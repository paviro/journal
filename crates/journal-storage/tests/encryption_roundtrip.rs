//! Encrypt → decrypt round-trip through the public store API. Encryption is
//! covered end-to-end by the CLI tests; this fills the decrypt gap: that
//! `decrypt_store` restores readable plaintext, drops the recipients file,
//! disables the identity, and leaves a backup.

use journal_storage::{EntryMetadata, JournalStore};

fn store_at(dir: &std::path::Path) -> JournalStore {
    JournalStore::new(
        dir.join("journals"),
        dir.join("recipients.txt"),
        dir.join("identity.age"),
    )
}

#[test]
fn decrypt_store_restores_plaintext_and_disables_encryption() {
    let dir = tempfile::tempdir().unwrap();

    let mut store = store_at(dir.path());
    store.ensure().unwrap();
    store.initialize_encryption("passphrase").unwrap();
    store.unlock("passphrase").unwrap();
    assert!(store.encryption_enabled());

    store.create_journal("diary").unwrap();
    let path = store
        .create_entry_with_body("diary", "# Secret\nhidden body", EntryMetadata::empty())
        .unwrap();
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("age"));

    let summary = store.decrypt_store().unwrap();
    assert!(summary.migrated_files >= 1);
    assert!(summary.backup_path.is_some_and(|p| p.exists()));
    assert!(summary.disabled_identity_file.exists());

    // Recipients gone → a fresh store treats everything as plaintext and reads it.
    assert!(!dir.path().join("recipients.txt").exists());
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
    store.initialize_encryption("passphrase").unwrap();

    // Never unlocked → decrypt refuses with the locked-identity error.
    let error = store.decrypt_store().unwrap_err();
    assert!(
        error
            .downcast_ref::<journal_storage::StorageError>()
            .is_some_and(|e| matches!(e, journal_storage::StorageError::LockedIdentity { .. }))
    );
}
