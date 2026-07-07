use super::*;
use crate::cipher::decrypt_bytes_with_identity;
use std::{fs, path::Path};
use tempfile::tempdir;

fn paths_in(dir: &Path) -> KeyPaths {
    KeyPaths::for_config(&dir.join("config.toml"), &dir.join("journals")).unwrap()
}

#[test]
fn passphrase_identity_round_trips_a_message() {
    let dir = tempdir().unwrap();
    let paths = paths_in(dir.path());

    initialize_store_identity(&paths, "laptop", Some(&SecretString::from("secret"))).unwrap();
    let unlocked = unlock_identity(&paths, Some(&SecretString::from("secret"))).unwrap();

    let ciphertext = encrypt_bytes(&paths, b"hello journal").unwrap();
    assert_eq!(
        decrypt_file_bytes_from(&unlocked, &ciphertext).unwrap(),
        b"hello journal"
    );
}

#[test]
fn initialize_store_identity_refuses_an_existing_roster() {
    let dir = tempdir().unwrap();
    let paths = paths_in(dir.path());

    initialize_store_identity(&paths, "laptop", None).unwrap();
    // A second genesis on the same roster would brick decryption; it must error.
    let err = initialize_store_identity(&paths, "laptop-again", None).unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn plaintext_identity_unlocks_without_a_passphrase() {
    let dir = tempdir().unwrap();
    let paths = paths_in(dir.path());

    initialize_store_identity(&paths, "phone", None).unwrap();
    let info = device_identity_info(&paths).unwrap().unwrap();
    assert!(!info.passphrase_protected);

    let unlocked = unlock_identity(&paths, None).unwrap();
    let ciphertext = encrypt_bytes(&paths, b"no passphrase").unwrap();
    assert_eq!(
        decrypt_file_bytes_from(&unlocked, &ciphertext).unwrap(),
        b"no passphrase"
    );
}

#[test]
fn two_recipients_both_decrypt_the_same_ciphertext() {
    let dir = tempdir().unwrap();
    let laptop = paths_in(dir.path());
    // A second device with its own identity file but the same shared store.
    let phone = KeyPaths::for_config(
        &dir.path().join("phone").join("config.toml"),
        &dir.path().join("journals"),
    )
    .unwrap();

    initialize_store_identity(&laptop, "laptop", Some(&SecretString::from("pw"))).unwrap();
    let laptop_id = unlock_identity(&laptop, Some(&SecretString::from("pw"))).unwrap();
    let phone_recipient = request_store_access(&phone, "phone", None).unwrap();
    add_recipient(&laptop, &laptop_id, &phone_recipient).unwrap();
    advance_trust_pins(&laptop).unwrap();

    let ciphertext = encrypt_bytes(&laptop, b"shared secret").unwrap();
    let phone_id = unlock_identity(&phone, None).unwrap();
    assert_eq!(
        decrypt_file_bytes_from(&laptop_id, &ciphertext).unwrap(),
        b"shared secret"
    );
    assert_eq!(
        decrypt_file_bytes_from(&phone_id, &ciphertext).unwrap(),
        b"shared secret"
    );
}

#[test]
fn pending_request_round_trips_and_clears() {
    let dir = tempdir().unwrap();
    let laptop = paths_in(dir.path());
    let phone = KeyPaths::for_config(
        &dir.path().join("phone").join("config.toml"),
        &dir.path().join("journals"),
    )
    .unwrap();

    initialize_store_identity(&laptop, "laptop", Some(&SecretString::from("pw"))).unwrap();
    request_store_access(&phone, "phone", None).unwrap();

    let pending = read_pending(&laptop).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].recipient.name, "phone");

    remove_pending(&laptop, &pending[0].id).unwrap();
    assert!(read_pending(&laptop).unwrap().is_empty());
}

#[test]
fn add_recipient_rejects_duplicate_key_and_name() {
    let dir = tempdir().unwrap();
    let paths = paths_in(dir.path());
    let recipient =
        initialize_store_identity(&paths, "laptop", Some(&SecretString::from("pw"))).unwrap();
    let identity = unlock_identity(&paths, Some(&SecretString::from("pw"))).unwrap();

    // Same key → rejected for the key clash.
    assert!(add_recipient(&paths, &identity, &recipient).is_err());
    // Same name, different (valid) key → rejected for the name clash.
    let same_name_new_key = Recipient {
        enc_key: "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsuaxjx".to_string(),
        ..recipient
    };
    assert!(add_recipient(&paths, &identity, &same_name_new_key).is_err());
}

#[test]
fn revoke_recipient_refuses_the_last_one() {
    let dir = tempdir().unwrap();
    let paths = paths_in(dir.path());
    initialize_store_identity(&paths, "laptop", Some(&SecretString::from("pw"))).unwrap();
    let identity = unlock_identity(&paths, Some(&SecretString::from("pw"))).unwrap();

    assert!(revoke_recipient(&paths, &identity, "laptop").is_err());
}

#[test]
fn set_identity_passphrase_toggles_protection_without_changing_the_key() {
    let dir = tempdir().unwrap();
    let paths = paths_in(dir.path());
    initialize_store_identity(&paths, "laptop", None).unwrap();
    let key = unlock_identity(&paths, None).unwrap().public_key();
    assert!(
        !device_identity_info(&paths)
            .unwrap()
            .unwrap()
            .passphrase_protected
    );

    // Add a passphrase.
    set_identity_passphrase(&paths, None, Some(&SecretString::from("pw"))).unwrap();
    assert!(
        device_identity_info(&paths)
            .unwrap()
            .unwrap()
            .passphrase_protected
    );
    assert_eq!(
        unlock_identity(&paths, Some(&SecretString::from("pw")))
            .unwrap()
            .public_key(),
        key
    );

    // The wrong current passphrase is rejected.
    assert!(set_identity_passphrase(&paths, Some(&SecretString::from("wrong")), None).is_err());

    // Remove the passphrase again; the keypair is unchanged throughout.
    set_identity_passphrase(&paths, Some(&SecretString::from("pw")), None).unwrap();
    assert!(
        !device_identity_info(&paths)
            .unwrap()
            .unwrap()
            .passphrase_protected
    );
    assert_eq!(unlock_identity(&paths, None).unwrap().public_key(), key);
}

#[test]
fn stored_identity_rejects_unknown_fields() {
    let dir = tempdir().unwrap();
    let paths = paths_in(dir.path());
    initialize_store_identity(&paths, "laptop", Some(&SecretString::from("secret"))).unwrap();

    let text = fs::read_to_string(&paths.identity_file).unwrap();
    fs::write(
        &paths.identity_file,
        format!("unexpected = \"unused\"\n{text}"),
    )
    .unwrap();

    assert!(unlock_identity(&paths, Some(&SecretString::from("secret"))).is_err());
}

/// Decrypt an in-memory ciphertext with an unlocked identity (test helper;
/// the production path decrypts files, not buffers).
fn decrypt_file_bytes_from(identity: &UnlockedIdentity, ciphertext: &[u8]) -> Result<Vec<u8>> {
    decrypt_bytes_with_identity(ciphertext, &identity.identity)
}
