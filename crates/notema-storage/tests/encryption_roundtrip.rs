//! Encrypt → decrypt round-trip through the public store API. Encryption is
//! covered end-to-end by the CLI tests; this fills the decrypt gap: that
//! `decrypt_store` restores readable plaintext, drops the recipients file,
//! disables the identity, and leaves a backup.

use notema_domain::Metadata;
use notema_encryption::{EncryptionError, SecretString};
use notema_storage::JournalStore;

fn store_at(dir: &std::path::Path) -> JournalStore {
    JournalStore::new(dir.join("journals"), dir)
}

fn pw(passphrase: &str) -> SecretString {
    SecretString::from(passphrase)
}

fn create_entry(store: &JournalStore, journal: &str, body: &str) -> std::path::PathBuf {
    store
        .create_entry(
            notema_storage::EntryDraft::new(journal, body, &Metadata::default()),
            notema_storage::EntryAssetOptions::default(),
        )
        .unwrap()
        .path
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
    let path = create_entry(&store, "diary", "# Secret\nhidden body");
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
            .join("devices.toml")
            .exists()
    );
    let plain = store_at(dir.path());
    assert!(!plain.encryption_enabled());
    let entries = plain.scan_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].body.contains("hidden body"));
    assert!(entries[0].path.to_string_lossy().ends_with(".md"));
}

#[test]
fn deleting_the_roster_with_encrypted_entries_fails_access_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = store_at(dir.path());
    store.ensure().unwrap();
    store
        .initialize_encryption("laptop", Some(&pw("passphrase")))
        .unwrap();
    store.unlock(Some(&pw("passphrase"))).unwrap();
    store.create_journal("diary").unwrap();
    create_entry(&store, "diary", "# Secret\nhidden body");

    // Simulate an attacker (or sync glitch) deleting only the roster from the
    // synced folder while the encrypted entry and this device's trust pins remain.
    std::fs::remove_file(
        dir.path()
            .join("journals")
            .join(".age")
            .join("devices.toml"),
    )
    .unwrap();

    // The store must refuse access rather than silently treat itself as plaintext
    // and start writing new entries in the clear.
    let reopened = store_at(dir.path());
    let error = match reopened.resolve_access() {
        Ok(_) => panic!("resolve_access must fail closed when the roster is gone"),
        Err(error) => error,
    };
    assert!(
        error
            .downcast_ref::<EncryptionError>()
            .is_some_and(|e| matches!(e, EncryptionError::RecipientsMissing { .. }))
    );
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
            .downcast_ref::<EncryptionError>()
            .is_some_and(|e| matches!(e, EncryptionError::Locked { .. }))
    );
}

#[test]
fn enable_encryption_rolls_back_root_and_local_key_state_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_at(dir.path());
    store.ensure().unwrap();
    store.create_journal("diary").unwrap();
    let path = create_entry(&store, "diary", "body before enable");
    let stem = notema_storage::entry_id(&path).unwrap();
    let assets = path.parent().unwrap().join(format!("{stem}.assets"));
    std::fs::create_dir_all(&assets).unwrap();
    let asset = assets.join("photo.png");
    let colliding_encrypted_asset = assets.join("photo.png.age");
    std::fs::write(&asset, "image").unwrap();
    std::fs::write(&colliding_encrypted_asset, "collision").unwrap();

    let error = store
        .enable_encryption("laptop", Some(&pw("pw")), |_, _| {})
        .unwrap_err();

    assert!(error.to_string().contains("restored unchanged"), "{error}");
    assert!(path.exists(), "plaintext entry should be restored");
    assert!(asset.exists(), "plaintext asset should be restored");
    assert!(
        colliding_encrypted_asset.exists(),
        "pre-existing collision should remain"
    );
    assert!(
        !dir.path()
            .join("journals")
            .join(".age")
            .join("devices.toml")
            .exists(),
        "new roster should be rolled back"
    );
    assert!(
        !dir.path().join("identity.toml").exists(),
        "new identity should be removed"
    );
    assert!(
        !dir.path().join("devices-trust.toml").exists(),
        "new trust pins should be removed"
    );
    let leftover_backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".backup-"));
    assert!(!leftover_backup, "rollback should consume the backup");
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
    let good = create_entry(&laptop, "diary", "keep me");
    let bad = create_entry(&laptop, "diary", "corrupt me");

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
    create_entry(&laptop, "diary", "body");

    // A pending join request left sitting in the synced key folder.
    let phone = JournalStore::new(dir.path().join("journals"), dir.path().join("phone"));
    phone.ensure().unwrap();
    phone.request_access("phone", None).unwrap();
    let age_dir = dir.path().join("journals").join(".age");
    assert!(age_dir.join("devices.toml").exists());
    assert!(!laptop.pending_requests().unwrap().is_empty());

    let summary = laptop.decrypt_store(|_, _| {}).unwrap();

    // Disabling encryption tears the whole key folder down, pending requests included.
    assert!(
        !age_dir.exists(),
        ".age folder should be gone after disable"
    );

    // The local trust pins are renamed aside (recoverable), not deleted — the
    // same treatment the identity gets.
    let trust_file = dir.path().join("devices-trust.toml");
    assert!(!trust_file.exists(), "trust pins should be renamed away");
    let retired_pins = summary
        .disabled_trust_file
        .expect("trust pins were present");
    assert!(retired_pins.exists());
    assert_eq!(
        retired_pins
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| { n.starts_with("devices-trust.disabled-") && n.ends_with(".toml") }),
        Some(true)
    );
}

#[test]
fn other_device_picks_up_a_remote_disable_and_retires_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let journals = dir.path().join("journals");

    // Laptop creates the encrypted store and writes an entry.
    let mut laptop = JournalStore::new(&journals, dir.path().join("laptop"));
    laptop.ensure().unwrap();
    laptop.initialize_encryption("laptop", None).unwrap();
    laptop.unlock(None).unwrap();
    laptop.create_journal("diary").unwrap();
    create_entry(&laptop, "diary", "shared body");

    // Phone joins, is approved, then unlocks so it pins the roster locally.
    let mut phone = JournalStore::new(&journals, dir.path().join("phone"));
    phone.ensure().unwrap();
    let phone_recipient = phone.request_access("phone", None).unwrap();
    laptop.add_recipient(phone_recipient, |_, _| {}).unwrap();
    phone.unlock(None).unwrap();

    let phone_identity = dir.path().join("phone").join("identity.toml");
    let phone_trust = dir.path().join("phone").join("devices-trust.toml");
    assert!(phone_identity.exists());
    assert!(phone_trust.exists());

    // Laptop disables encryption: entries return to plaintext and the synced
    // roster is removed. Over a shared folder that is all the phone observes.
    laptop.decrypt_store(|_, _| {}).unwrap();
    assert!(!journals.join(".age").join("devices.toml").exists());

    // The phone reopens. Reconciliation notices the roster it had pinned is gone
    // with no encrypted entries left, so it retires its own key and pins.
    let phone = JournalStore::new(&journals, dir.path().join("phone"));
    phone.ensure().unwrap();
    assert!(phone.reconcile_disabled_encryption().unwrap());

    assert!(!phone_identity.exists(), "phone identity should be retired");
    assert!(!phone_trust.exists(), "phone trust pins should be retired");
    assert!(!phone.encryption_enabled());
    assert!(!phone.unlock_available());

    let names: Vec<String> = std::fs::read_dir(dir.path().join("phone"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("identity.disabled-") && n.ends_with(".toml")),
        "retired identity copy should remain: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("devices-trust.disabled-") && n.ends_with(".toml")),
        "retired trust-pin copy should remain: {names:?}"
    );

    // And the phone still reads the now-plaintext shared entry.
    let entries = phone.scan_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].body.contains("shared body"));
}

#[test]
fn revoked_device_retires_its_identity_but_keeps_trust_pins() {
    let dir = tempfile::tempdir().unwrap();
    let journals = dir.path().join("journals");

    // Laptop creates the encrypted store; phone joins, is approved, and pins the
    // roster locally by unlocking.
    let mut laptop = JournalStore::new(&journals, dir.path().join("laptop"));
    laptop.ensure().unwrap();
    laptop.initialize_encryption("laptop", None).unwrap();
    laptop.unlock(None).unwrap();
    laptop.create_journal("diary").unwrap();
    create_entry(&laptop, "diary", "shared body");

    let mut phone = JournalStore::new(&journals, dir.path().join("phone"));
    phone.ensure().unwrap();
    let phone_recipient = phone.request_access("phone", None).unwrap();
    laptop.add_recipient(phone_recipient, |_, _| {}).unwrap();
    phone.unlock(None).unwrap();

    let phone_identity = dir.path().join("phone").join("identity.toml");
    let phone_trust = dir.path().join("phone").join("devices-trust.toml");
    assert!(phone_identity.exists());
    assert!(phone_trust.exists());

    // Laptop revokes the phone: the store stays encrypted (for the laptop) but
    // the phone is no longer a recipient.
    laptop.revoke_recipient("phone", |_, _| {}).unwrap();
    assert!(journals.join(".age").join("devices.toml").exists());

    // The phone reopens, is not a recipient, and has nothing queued — the caller
    // retires its now-dead key.
    let phone = JournalStore::new(&journals, dir.path().join("phone"));
    phone.ensure().unwrap();
    let retired = phone
        .retire_revoked_identity()
        .unwrap()
        .expect("identity retired");
    assert!(retired.exists());

    // The identity is renamed aside; the trust pins are deliberately kept so a
    // re-enroll still validates against the unchanged roster genesis.
    assert!(!phone_identity.exists(), "phone identity should be retired");
    assert!(!phone.unlock_available());
    assert!(phone_trust.exists(), "phone trust pins should be kept");

    let names: Vec<String> = std::fs::read_dir(dir.path().join("phone"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("identity.disabled-") && n.ends_with(".toml")),
        "retired identity copy should remain: {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with("devices-trust.disabled-")),
        "trust pins should not be retired: {names:?}"
    );
}

#[test]
fn remote_disable_reconcile_holds_off_while_entries_are_still_encrypted() {
    // A half-synced disable: the roster deletion arrived before the plaintext
    // entry conversions. The device must keep its key to read what's still
    // encrypted, so reconciliation holds off.
    let dir = tempfile::tempdir().unwrap();
    let journals = dir.path().join("journals");
    let mut laptop = JournalStore::new(&journals, dir.path().join("laptop"));
    laptop.ensure().unwrap();
    laptop.initialize_encryption("laptop", None).unwrap();
    laptop.unlock(None).unwrap();
    laptop.create_journal("diary").unwrap();
    create_entry(&laptop, "diary", "still secret");

    // Remove only the roster, leaving the encrypted entry in place.
    std::fs::remove_file(journals.join(".age").join("devices.toml")).unwrap();
    let identity = dir.path().join("laptop").join("identity.toml");
    let trust = dir.path().join("laptop").join("devices-trust.toml");
    assert!(trust.exists());

    assert!(!laptop.reconcile_disabled_encryption().unwrap());

    assert!(
        identity.exists(),
        "key must survive while entries are encrypted"
    );
    assert!(
        trust.exists(),
        "pins must survive while entries are encrypted"
    );
}

#[test]
fn revoke_recipient_refuses_own_device_even_after_rename() {
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

    let error = laptop.revoke_recipient("renamed", |_, _| {}).unwrap_err();
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
    create_entry(&laptop, "diary", "history");

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
    create_entry(&laptop, "diary", "secret");

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
        notema_domain::EntryEncryptionState::EncryptedLocked
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
    let path = create_entry(&store, "diary", "before rotation");
    let old_key = store.public_recipient().unwrap();

    // Two passes re-encrypt the single entry, so at least two files migrate.
    let summary = store.rotate_identity(Some(&pw("pw")), |_, _| {}).unwrap();
    assert!(summary.migrated_files >= 2);

    // The device is now the sole recipient under a fresh key.
    let recipients = store.recipients().unwrap();
    assert_eq!(recipients.len(), 1);
    assert_ne!(recipients[0].encryption_key, old_key);

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

/// The byte-level encrypted store-file API the FUSE mount is built on:
/// rewriting an entry re-encrypts it verbatim (byte-for-byte round-trip), the
/// file stays encrypted on disk, and an encrypted asset written alongside it
/// round-trips as raw bytes too.
#[test]
fn write_store_file_reencrypts_entries_and_assets() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = store_at(dir.path());
    store.ensure().unwrap();
    store
        .initialize_encryption("laptop", Some(&pw("pw")))
        .unwrap();
    store.unlock(Some(&pw("pw"))).unwrap();
    store.create_journal("diary").unwrap();
    let path = create_entry(&store, "diary", "original body");
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("age"));
    assert!(store.encrypts_new_files());

    // Rewrite the entry through the byte-level API, as the mount's commit does.
    let edited =
        b"+++\nschema_version = 1\n\n[entry]\ntags = [\"edited\"]\n+++\n\nnew body through the mount\n";
    store
        .write_store_file(&path, notema_storage::StoreFileEncoding::Encrypted, edited)
        .unwrap();
    assert_eq!(
        store
            .read_store_file(&path, notema_storage::StoreFileEncoding::Encrypted)
            .unwrap(),
        edited
    );

    // Still encrypted on disk: the raw bytes are age ciphertext, not plaintext.
    let raw = std::fs::read(&path).unwrap();
    assert!(
        raw.windows(b"new body".len()).all(|w| w != b"new body"),
        "plaintext must not appear in the on-disk ciphertext"
    );
    assert!(
        raw.starts_with(b"age-encryption.org/"),
        "on-disk file should be a binary age file"
    );

    // An encrypted asset written next to the entry round-trips as raw bytes.
    let asset = path.with_extension("").with_extension("jpg.age");
    let bytes: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    store
        .write_store_file(&asset, notema_storage::StoreFileEncoding::Encrypted, &bytes)
        .unwrap();
    assert_eq!(
        store
            .read_store_file(&asset, notema_storage::StoreFileEncoding::Encrypted)
            .unwrap(),
        bytes
    );
    assert!(
        std::fs::read(&asset)
            .unwrap()
            .starts_with(b"age-encryption.org/"),
        "asset should be encrypted on disk"
    );
}

// --- roster integrity: the whole point of the signed device log ---------------
//
// A folder-write attacker can rewrite `.age/devices.toml`. These assert the store
// refuses to hand back a recipient set (fails closed) rather than silently
// trusting a tampered roster — which would leak future entries to an injected key.

fn devices_file(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("journals").join(".age").join("devices.toml")
}

/// A hand-appended recipient (no valid signature by a trusted device) is rejected,
/// so the store won't encrypt to the attacker's key.
#[test]
fn injected_recipient_in_roster_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = store_at(dir.path());
    store.ensure().unwrap();
    store.initialize_encryption("laptop", None).unwrap();
    store.unlock(None).unwrap();
    assert!(store.recipients().is_ok());

    // Attacker appends themselves as a recipient by editing the synced file.
    let path = devices_file(dir.path());
    let mut roster = std::fs::read_to_string(&path).unwrap();
    roster.push_str(
        "\n[[operation]]\nseq = 1\nprev_hash = \"\"\nkind = \"add\"\nname = \"attacker\"\n\
         encryption_key = \"age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsuaxjx\"\n\
         signing_key = \"ed25519:0000000000000000000000000000000000000000000000000000000000000000\"\n\
         authorized_by = \"ed25519:0000000000000000000000000000000000000000000000000000000000000000\"\n\
         sig = \"00\"\n",
    );
    std::fs::write(&path, roster).unwrap();

    let error = store.recipients().unwrap_err().to_string();
    assert!(error.contains("roster failed verification"), "{error}");
}

/// Flipping a signed field of the genesis op breaks its signature and is rejected.
#[test]
fn tampered_roster_field_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = store_at(dir.path());
    store.ensure().unwrap();
    store.initialize_encryption("laptop", None).unwrap();
    store.unlock(None).unwrap();

    let path = devices_file(dir.path());
    let roster = std::fs::read_to_string(&path)
        .unwrap()
        .replace("name = \"laptop\"", "name = \"hacker\"");
    std::fs::write(&path, roster).unwrap();

    let error = store.recipients().unwrap_err().to_string();
    assert!(error.contains("roster failed verification"), "{error}");
}

/// A roster rewound below a state this device already pinned (e.g. the sync host
/// hiding an added device) is rejected as a rollback.
#[test]
fn rolled_back_roster_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut laptop = store_at(dir.path());
    laptop.ensure().unwrap();
    laptop.initialize_encryption("laptop", None).unwrap();
    laptop.unlock(None).unwrap();

    // Snapshot the genesis-only roster, then genuinely add a second device so the
    // laptop pins the new head.
    let path = devices_file(dir.path());
    let genesis_only = std::fs::read_to_string(&path).unwrap();

    let phone = JournalStore::new(dir.path().join("journals"), dir.path().join("phone"));
    let phone_recipient = phone.request_access("phone", None).unwrap();
    laptop.add_recipient(phone_recipient, |_, _| {}).unwrap();
    assert_eq!(laptop.recipients().unwrap().len(), 2);

    // The sync host serves the old, pre-add roster back: the pinned head is gone.
    std::fs::write(&path, genesis_only).unwrap();
    let error = laptop.recipients().unwrap_err().to_string();
    assert!(error.contains("roster failed verification"), "{error}");
}

/// Creating an entry with assets never needs the unlocked identity: the body is
/// ingested from memory (no read-back), and both the asset files and the
/// rewritten entry are encrypted with the recipients roster alone.
#[test]
fn create_entry_ingests_assets_on_a_locked_encrypted_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut setup = store_at(dir.path());
    setup.ensure().unwrap();
    setup.initialize_encryption("laptop", None).unwrap();

    let image = dir.path().join("pic.jpeg");
    std::fs::write(&image, b"fake jpeg bytes").unwrap();

    // A fresh store that is never unlocked.
    let locked = store_at(dir.path());
    assert!(!locked.is_unlocked());
    let body = format!("Look:\n\n![pic]({})", image.display());
    let metadata = Metadata::default();
    let mut draft = notema_storage::EntryDraft::new("diary", &body, &metadata);
    draft.writing_seconds = Some(12);
    let created = locked
        .create_entry(draft, notema_storage::EntryAssetOptions::default())
        .unwrap();

    assert_eq!(created.assets.stored, 1);
    assert!(created.assets.failed.is_empty());
    assert!(created.path.to_string_lossy().ends_with(".md.age"));

    // The rewritten body references the entry's own asset folder, not the source.
    setup.unlock(None).unwrap();
    let entries = setup.scan_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].body.contains(".assets/"), "{}", entries[0].body);
    assert!(!entries[0].body.contains("pic.jpeg"));
    let content = setup.read_entry_content(&entries[0].path).unwrap();
    assert!(content.contains("writing_seconds = 12"), "{content}");
}
