use crate::identity::UnlockedIdentity;
use crate::recipients::{Recipient, read_recipients};
use crate::{EncryptionError, KeyPaths, Result};
use age::x25519;
use std::{fs, io::Write, path::Path, str::FromStr};

pub fn encrypt_to_file(paths: &KeyPaths, plaintext: &[u8], output: &Path) -> Result<()> {
    EncryptionRecipients::for_store(paths)?.encrypt_to_file(plaintext, output)?;
    Ok(())
}

/// Decrypt an encrypted file into memory. Used both for reading encrypted entry
/// text and for viewing encrypted binary assets (e.g. images) without ever
/// writing a plaintext copy to disk.
pub fn decrypt_file_bytes(identity: &UnlockedIdentity, input: &Path) -> Result<Vec<u8>> {
    let ciphertext = fs::read(input)?;
    decrypt_bytes_with_identity(&ciphertext, &identity.identity)
}

/// Encrypt bytes to every store recipient.
pub fn encrypt_bytes(paths: &KeyPaths, plaintext: &[u8]) -> Result<Vec<u8>> {
    EncryptionRecipients::for_store(paths)?.encrypt(plaintext)
}

/// Encrypt a freshly created entry to every store recipient, plus this device's
/// own key when unlocked — so the authoring device can always re-read what it
/// wrote, even a joining device whose key isn't yet an approved recipient.
pub fn encrypt_new_entry(
    paths: &KeyPaths,
    plaintext: &[u8],
    identity: Option<&UnlockedIdentity>,
) -> Result<Vec<u8>> {
    EncryptionRecipients::for_new_entry(paths, identity)?.encrypt(plaintext)
}

/// Store recipients parsed into age recipient keys, ready for repeated
/// encryptions without rereading the roster.
pub struct EncryptionRecipients {
    recipients: Vec<x25519::Recipient>,
}

impl EncryptionRecipients {
    pub fn for_store(paths: &KeyPaths) -> Result<Self> {
        Ok(Self {
            recipients: recipient_keys(&read_recipients(paths)?)?,
        })
    }

    pub fn for_new_entry(paths: &KeyPaths, identity: Option<&UnlockedIdentity>) -> Result<Self> {
        let mut recipients = recipient_keys(&read_recipients(paths)?)?;
        if let Some(identity) = identity {
            let own = identity.recipient();
            if !recipients.iter().any(|r| r.to_string() == own.to_string()) {
                recipients.push(own);
            }
        }
        Ok(Self { recipients })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        encrypt_to_recipients(&self.recipients, plaintext)
    }

    pub fn encrypt_to_file(&self, plaintext: &[u8], output: &Path) -> Result<()> {
        fs::write(output, self.encrypt(plaintext)?)?;
        Ok(())
    }
}

fn recipient_keys(recipients: &[Recipient]) -> Result<Vec<x25519::Recipient>> {
    if recipients.is_empty() {
        return Err(EncryptionError::NoRecipients);
    }
    recipients
        .iter()
        .map(|recipient| {
            x25519::Recipient::from_str(&recipient.enc_key).map_err(|_| {
                EncryptionError::InvalidRecipientKey {
                    key: recipient.enc_key.clone(),
                }
            })
        })
        .collect()
}

pub(crate) fn encrypt_to_recipients(
    recipients: &[x25519::Recipient],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let refs: Vec<&dyn age::Recipient> = recipients
        .iter()
        .map(|recipient| recipient as &dyn age::Recipient)
        .collect();
    let encryptor = age::Encryptor::with_recipients(refs.into_iter())?;
    let mut ciphertext = Vec::with_capacity(plaintext.len());
    let mut writer = encryptor.wrap_output(&mut ciphertext)?;
    writer.write_all(plaintext)?;
    writer.finish()?;
    Ok(ciphertext)
}

pub(crate) fn decrypt_bytes_with_identity(
    ciphertext: &[u8],
    identity: &x25519::Identity,
) -> Result<Vec<u8>> {
    Ok(age::decrypt(identity, ciphertext)?)
}
