use crate::AppResult;
use age::{
    secrecy::{ExposeSecret, SecretString},
    x25519,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionPaths {
    pub config_dir: PathBuf,
    pub recipients_file: PathBuf,
    pub identity_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredIdentity {
    encrypted_identity: Vec<u8>,
}

pub struct UnlockedIdentity {
    identity: x25519::Identity,
}

impl EncryptionPaths {
    pub fn for_config(config_path: &Path, journal_root: &Path) -> AppResult<Self> {
        let config_dir = config_path
            .parent()
            .ok_or("config path has no parent directory")?;
        Ok(Self {
            recipients_file: journal_root.join("recipients.txt"),
            identity_file: config_dir.join("identity.age"),
            config_dir: config_dir.to_path_buf(),
        })
    }
}

pub fn should_encrypt(paths: &EncryptionPaths) -> bool {
    paths.recipients_file.exists()
}

pub fn can_decrypt(paths: &EncryptionPaths) -> bool {
    paths.identity_file.exists()
}

pub fn public_recipient(paths: &EncryptionPaths) -> AppResult<String> {
    let recipient_text = fs::read_to_string(&paths.recipients_file)?;
    Ok(first_recipient(&recipient_text)?.to_string())
}

pub fn generate_identity_store(paths: &EncryptionPaths, passphrase: &str) -> AppResult<String> {
    if passphrase.is_empty() {
        return Err("encryption passphrase cannot be empty".into());
    }

    let identity = x25519::Identity::generate();
    write_identity_store(paths, &identity, passphrase)
}

pub fn unlock_identity(paths: &EncryptionPaths, passphrase: &str) -> AppResult<UnlockedIdentity> {
    let identity = decrypt_identity(paths, passphrase)?;

    let encrypted = encrypt_bytes(paths, b"journal identity check")?;
    let decrypted = decrypt_bytes_with_identity(&encrypted, &identity)?;
    if decrypted != b"journal identity check" {
        return Err("journal encryption identity check failed".into());
    }

    Ok(UnlockedIdentity { identity })
}

pub fn encrypt_file(paths: &EncryptionPaths, input: &Path, output: &Path) -> AppResult<()> {
    let plaintext = fs::read(input)?;
    fs::write(output, encrypt_bytes(paths, &plaintext)?)?;
    Ok(())
}

pub fn encrypt_to_file(paths: &EncryptionPaths, plaintext: &[u8], output: &Path) -> AppResult<()> {
    fs::write(output, encrypt_bytes(paths, plaintext)?)?;
    Ok(())
}

pub fn decrypt_file(identity: &UnlockedIdentity, input: &Path, output: &Path) -> AppResult<()> {
    let plaintext = decrypt_file_bytes(identity, input)?;
    fs::write(output, plaintext)?;
    Ok(())
}

pub fn decrypt_to_string(identity: &UnlockedIdentity, input: &Path) -> AppResult<String> {
    Ok(String::from_utf8(decrypt_file_bytes(identity, input)?)?)
}

fn decrypt_file_bytes(identity: &UnlockedIdentity, input: &Path) -> AppResult<Vec<u8>> {
    let ciphertext = fs::read(input)?;
    decrypt_bytes_with_identity(&ciphertext, &identity.identity)
}

pub fn encrypt_bytes(paths: &EncryptionPaths, plaintext: &[u8]) -> AppResult<Vec<u8>> {
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
    paths: &EncryptionPaths,
    identity: &x25519::Identity,
    passphrase: &str,
) -> AppResult<String> {
    fs::create_dir_all(&paths.config_dir)?;
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

fn decrypt_identity(paths: &EncryptionPaths, passphrase: &str) -> AppResult<x25519::Identity> {
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

fn write_public_recipient(paths: &EncryptionPaths, recipient: &str) -> AppResult<()> {
    if let Some(parent) = paths.recipients_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&paths.recipients_file, format!("{recipient}\n"))?;
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
            EncryptionPaths::for_config(&dir.path().join("config.toml"), &journal_root).unwrap();

        let recipient = generate_identity_store(&paths, "secret").unwrap();
        let unlocked = unlock_identity(&paths, "secret").unwrap();
        let plain = dir.path().join("plain.md");
        let encrypted = dir.path().join("plain.md.age");
        let decrypted = dir.path().join("decrypted.md");
        fs::write(&plain, "hello journal\n").unwrap();

        encrypt_file(&paths, &plain, &encrypted).unwrap();
        decrypt_file(&unlocked, &encrypted, &decrypted).unwrap();

        assert!(!recipient.is_empty());
        assert_eq!(fs::read_to_string(decrypted).unwrap(), "hello journal\n");
    }

    #[test]
    fn stored_identity_rejects_unknown_fields() {
        let dir = tempdir().unwrap();
        let journal_root = dir.path().join("journals");
        let paths =
            EncryptionPaths::for_config(&dir.path().join("config.toml"), &journal_root).unwrap();

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
