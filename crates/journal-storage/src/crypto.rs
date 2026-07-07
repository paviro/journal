use crate::{AppResult, JournalStorePaths};
use age::{
    secrecy::{ExposeSecret, SecretString},
    x25519,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    str::FromStr,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredIdentity {
    encrypted_identity: Vec<u8>,
}

#[derive(Clone)]
pub struct UnlockedIdentity {
    identity: x25519::Identity,
}

pub fn has_recipients_file(paths: &JournalStorePaths) -> bool {
    paths.recipients_file.exists()
}

pub fn has_identity_file(paths: &JournalStorePaths) -> bool {
    paths.identity_file.exists()
}

pub fn public_recipient(paths: &JournalStorePaths) -> AppResult<String> {
    let recipient_text = fs::read_to_string(&paths.recipients_file)?;
    Ok(first_recipient(&recipient_text)?.to_string())
}

pub fn generate_identity_store(paths: &JournalStorePaths, passphrase: &str) -> AppResult<String> {
    if passphrase.is_empty() {
        return Err("encryption passphrase cannot be empty".into());
    }

    let identity = x25519::Identity::generate();
    write_identity_store(paths, &identity, passphrase)
}

pub fn unlock_identity(paths: &JournalStorePaths, passphrase: &str) -> AppResult<UnlockedIdentity> {
    let identity = decrypt_identity(paths, passphrase)?;

    let encrypted = encrypt_bytes(paths, b"journal identity check")?;
    let decrypted = decrypt_bytes_with_identity(&encrypted, &identity)?;
    if decrypted != b"journal identity check" {
        return Err("journal encryption identity check failed".into());
    }

    Ok(UnlockedIdentity { identity })
}

pub fn encrypt_to_file(
    paths: &JournalStorePaths,
    plaintext: &[u8],
    output: &Path,
) -> AppResult<()> {
    fs::write(output, encrypt_bytes(paths, plaintext)?)?;
    Ok(())
}

/// Decrypt an encrypted file into memory. Used both for reading encrypted entry
/// text and for viewing encrypted binary assets (e.g. images) without ever
/// writing a plaintext copy to disk.
pub fn decrypt_file_bytes(identity: &UnlockedIdentity, input: &Path) -> AppResult<Vec<u8>> {
    let ciphertext = fs::read(input)?;
    decrypt_bytes_with_identity(&ciphertext, &identity.identity)
}

pub fn encrypt_bytes(paths: &JournalStorePaths, plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let recipient_text = fs::read_to_string(&paths.recipients_file)?;
    let recipient = first_recipient(&recipient_text)?;
    let recipient = x25519::Recipient::from_str(recipient)?;
    Ok(age::encrypt(&recipient, plaintext)?)
}

fn decrypt_bytes_with_identity(
    ciphertext: &[u8],
    identity: &x25519::Identity,
) -> AppResult<Vec<u8>> {
    Ok(age::decrypt(identity, ciphertext)?)
}

fn write_identity_store(
    paths: &JournalStorePaths,
    identity: &x25519::Identity,
    passphrase: &str,
) -> AppResult<String> {
    let recipient = identity.to_public().to_string();

    write_public_recipient(paths, &recipient)?;
    let stored = StoredIdentity {
        encrypted_identity: encrypt_identity(identity, passphrase)?,
    };
    write_private_file(
        &paths.identity_file,
        toml::to_string_pretty(&stored)?.as_bytes(),
    )?;

    Ok(recipient)
}

fn encrypt_identity(identity: &x25519::Identity, passphrase: &str) -> AppResult<Vec<u8>> {
    let passphrase = SecretString::from(passphrase.to_string());
    let recipient = age::scrypt::Recipient::new(passphrase);
    Ok(age::encrypt(
        &recipient,
        identity.to_string().expose_secret().as_bytes(),
    )?)
}

fn decrypt_identity(paths: &JournalStorePaths, passphrase: &str) -> AppResult<x25519::Identity> {
    let passphrase = SecretString::from(passphrase.to_string());
    let identity = age::scrypt::Identity::new(passphrase);
    let stored = read_stored_identity(&paths.identity_file)?;
    let plaintext = age::decrypt(&identity, &stored.encrypted_identity)?;
    let text = String::from_utf8(plaintext)?;
    Ok(x25519::Identity::from_str(text.trim())?)
}

fn read_stored_identity(path: &Path) -> AppResult<StoredIdentity> {
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

/// Comment header written above the recipient. `first_recipient` skips `#`
/// lines, so this stays parse-safe while telling anyone who opens the file that
/// it is managed by the app and holds a non-secret public key.
const RECIPIENTS_HEADER: &str = "\
# Managed by journal — do not edit or delete.
# This is your age encryption recipient (a public key, not a secret).
# New entries are encrypted to it; losing it stops further encryption.
";

fn write_public_recipient(paths: &JournalStorePaths, recipient: &str) -> AppResult<()> {
    if let Some(parent) = paths.recipients_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &paths.recipients_file,
        format!("{RECIPIENTS_HEADER}{recipient}\n"),
    )?;
    Ok(())
}

fn first_recipient(recipients: &str) -> AppResult<&str> {
    recipients
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or_else(|| "journal encryption recipients file is empty".into())
}

fn write_private_file(path: &Path, content: &[u8]) -> AppResult<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn generated_identity_store_encrypts_and_decrypts_messages() {
        let dir = tempdir().unwrap();
        let journal_root = dir.path().join("journals");
        let paths =
            JournalStorePaths::for_config(&dir.path().join("config.toml"), &journal_root).unwrap();

        let recipient = generate_identity_store(&paths, "secret").unwrap();
        let unlocked = unlock_identity(&paths, "secret").unwrap();
        let plain = dir.path().join("plain.md");
        let encrypted = dir.path().join("plain.md.age");
        let decrypted = dir.path().join("decrypted.md");
        fs::write(&plain, "hello journal\n").unwrap();

        encrypt_to_file(&paths, &fs::read(&plain).unwrap(), &encrypted).unwrap();
        fs::write(
            &decrypted,
            decrypt_file_bytes(&unlocked, &encrypted).unwrap(),
        )
        .unwrap();

        assert!(!recipient.is_empty());
        assert_eq!(fs::read_to_string(decrypted).unwrap(), "hello journal\n");
    }

    #[test]
    fn stored_identity_rejects_unknown_fields() {
        let dir = tempdir().unwrap();
        let journal_root = dir.path().join("journals");
        let paths =
            JournalStorePaths::for_config(&dir.path().join("config.toml"), &journal_root).unwrap();

        generate_identity_store(&paths, "secret").unwrap();
        let text = fs::read_to_string(&paths.identity_file).unwrap();
        fs::write(
            &paths.identity_file,
            format!("unexpected = \"unused\"\n{text}"),
        )
        .unwrap();

        assert!(unlock_identity(&paths, "secret").is_err());
    }
}
