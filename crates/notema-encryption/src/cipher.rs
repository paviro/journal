use crate::identity::UnlockedIdentity;
use crate::recipients::{Recipient, read_recipients};
use crate::{EncryptionError, KeyPaths, Result};
use age::x25519;
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
    str::FromStr,
    string::FromUtf8Error,
};
use zeroize::Zeroizing;

/// Decrypted bytes. The inner buffer is zeroized on drop and intentionally does
/// not implement `Clone`, `Debug`, `Display`, or serde traits.
pub struct PlaintextBytes(Zeroizing<Vec<u8>>);

impl PlaintextBytes {
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn copy_from_slice(bytes: &[u8]) -> Self {
        Self::from_vec(bytes.to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_ref()
    }

    pub fn copy_to_vec(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }

    /// Convert decrypted UTF-8 bytes into a `String`. The returned `String` is
    /// plaintext and is not zeroized by this wrapper.
    pub fn into_string(self) -> std::result::Result<String, FromUtf8Error> {
        String::from_utf8(self.copy_to_vec())
    }
}

impl AsRef<[u8]> for PlaintextBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Encrypted age payload bytes. Kept distinct from plaintext so callers cannot
/// accidentally pass decrypted data where ciphertext is expected, or vice versa.
pub struct CiphertextBytes(Vec<u8>);

impl CiphertextBytes {
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

impl AsRef<[u8]> for CiphertextBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

pub fn encrypt_to_file(paths: &KeyPaths, plaintext: &PlaintextBytes, output: &Path) -> Result<()> {
    EncryptionRecipients::for_store(paths)?.encrypt_to_file(plaintext, output)?;
    Ok(())
}

/// Decrypt an encrypted file into memory. Used both for reading encrypted entry
/// text and for viewing encrypted binary assets (e.g. images) without ever
/// writing a plaintext copy to disk.
pub fn decrypt_file_bytes(identity: &UnlockedIdentity, input: &Path) -> Result<PlaintextBytes> {
    let ciphertext = CiphertextBytes::from_vec(fs::read(input)?);
    decrypt_bytes_with_identity(&ciphertext, &identity.identity)
}

/// Encrypt bytes to every store recipient.
pub fn encrypt_bytes(paths: &KeyPaths, plaintext: &PlaintextBytes) -> Result<CiphertextBytes> {
    EncryptionRecipients::for_store(paths)?.encrypt(plaintext)
}

/// Encrypt a freshly created entry to every store recipient, plus this device's
/// own key when unlocked — so the authoring device can always re-read what it
/// wrote, even a joining device whose key isn't yet an approved recipient.
pub fn encrypt_new_entry(
    paths: &KeyPaths,
    plaintext: &PlaintextBytes,
    identity: Option<&UnlockedIdentity>,
) -> Result<CiphertextBytes> {
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

    pub fn encrypt(&self, plaintext: &PlaintextBytes) -> Result<CiphertextBytes> {
        encrypt_to_recipients(&self.recipients, plaintext)
    }

    /// Encrypt a stream to every store recipient without buffering the full
    /// plaintext or ciphertext in memory.
    pub fn encrypt_reader<R, W>(&self, plaintext: R, output: W) -> Result<W>
    where
        R: Read,
        W: Write,
    {
        encrypt_reader_to_recipients(&self.recipients, plaintext, output)
    }

    pub fn encrypt_to_file(&self, plaintext: &PlaintextBytes, output: &Path) -> Result<()> {
        let ciphertext = self.encrypt(plaintext)?;
        crate::files::atomic_write(output, ciphertext.as_bytes())
    }
}

fn recipient_keys(recipients: &[Recipient]) -> Result<Vec<x25519::Recipient>> {
    if recipients.is_empty() {
        return Err(EncryptionError::NoRecipients);
    }
    recipients
        .iter()
        .map(|recipient| {
            x25519::Recipient::from_str(&recipient.encryption_key).map_err(|_| {
                EncryptionError::InvalidRecipientKey {
                    key: recipient.encryption_key.clone(),
                }
            })
        })
        .collect()
}

pub(crate) fn encrypt_to_recipients(
    recipients: &[x25519::Recipient],
    plaintext: &PlaintextBytes,
) -> Result<CiphertextBytes> {
    let mut ciphertext = Vec::with_capacity(plaintext.as_bytes().len());
    encrypt_reader_to_recipients(recipients, plaintext.as_bytes(), &mut ciphertext)?;
    Ok(CiphertextBytes::from_vec(ciphertext))
}

fn encrypt_reader_to_recipients<R, W>(
    recipients: &[x25519::Recipient],
    mut plaintext: R,
    output: W,
) -> Result<W>
where
    R: Read,
    W: Write,
{
    let refs: Vec<&dyn age::Recipient> = recipients
        .iter()
        .map(|recipient| recipient as &dyn age::Recipient)
        .collect();
    let encryptor = age::Encryptor::with_recipients(refs.into_iter())?;
    let mut writer = encryptor.wrap_output(output)?;
    io::copy(&mut plaintext, &mut writer)?;
    Ok(writer.finish()?)
}

pub(crate) fn decrypt_bytes_with_identity(
    ciphertext: &CiphertextBytes,
    identity: &x25519::Identity,
) -> Result<PlaintextBytes> {
    Ok(PlaintextBytes::from_vec(age::decrypt(
        identity,
        ciphertext.as_bytes(),
    )?))
}
