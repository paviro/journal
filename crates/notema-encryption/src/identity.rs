use crate::files::write_private_file;
use crate::signing::{generate_signing_key, signing_public};
use crate::{EncryptionError, KeyPaths, Recipient, Result, cipher, recipients};
use age::secrecy::{ExposeSecret, SecretString};
use age::x25519;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, str::FromStr};
use zeroize::Zeroizing;

/// This device's decrypted keypair: the age identity that reads encrypted
/// entries and the Ed25519 signing key that authorizes roster ops.
#[derive(Clone)]
pub struct UnlockedIdentity {
    pub(crate) identity: x25519::Identity,
    pub(crate) signing: SigningKey,
}

impl UnlockedIdentity {
    pub(crate) fn recipient(&self) -> x25519::Recipient {
        self.identity.to_public()
    }

    /// This identity's age public key string, for matching against recipients.
    pub fn public_key(&self) -> String {
        self.identity.to_public().to_string()
    }

    /// This device's Ed25519 signing public key, `ed25519:<hex>`.
    pub fn signing_public(&self) -> String {
        signing_public(&self.signing)
    }
}

/// The non-secret facts about this device's stored identity, readable without a
/// passphrase: how it labels itself and whether unlocking needs a passphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentityInfo {
    pub name: String,
    pub passphrase_protected: bool,
}

/// The secret key material for a device, serialized inside the (optionally
/// scrypt-wrapped) `identity.toml`: the age private key and the Ed25519 signing
/// seed. Kept together so both are protected by the same passphrase choice.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretBundle {
    x25519: Zeroizing<String>,
    ed25519: Zeroizing<String>,
}

/// How this device's secret bundle is stored, per its passphrase choice. The
/// on-disk file carries exactly one of the two forms; this enum makes that
/// either/or explicit in memory (see [`StoredIdentityWire`]).
enum KeyMaterial {
    /// scrypt-wrapped bundle as age ASCII armor, opened with a passphrase. The
    /// armor is a standalone age file, so recovery is possible with the `age` CLI.
    Encrypted(Zeroizing<String>),
    /// plaintext bundle, stored mode 0600 and opened without a passphrase.
    Plain(Zeroizing<String>),
}

/// This device's stored identity: the label it stores itself under and its key
/// material. Deserialized from [`StoredIdentityWire`], which enforces that
/// exactly one key form is present.
#[derive(Deserialize)]
#[serde(try_from = "StoredIdentityWire")]
struct StoredIdentity {
    device_name: String,
    key: KeyMaterial,
}

/// The on-disk shape of `identity.toml`: `device_name` plus exactly one of the
/// scrypt-wrapped or plaintext key fields. `encrypted_keys` holds age ASCII armor
/// so the secret is recoverable with the `age` CLI; [`StoredIdentity`] is the
/// validated in-memory view.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredIdentityWire {
    device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encrypted_keys: Option<Zeroizing<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plain_keys: Option<Zeroizing<String>>,
}

impl TryFrom<StoredIdentityWire> for StoredIdentity {
    type Error = &'static str;

    fn try_from(wire: StoredIdentityWire) -> std::result::Result<Self, Self::Error> {
        let key = match (wire.encrypted_keys, wire.plain_keys) {
            (Some(armor), None) => KeyMaterial::Encrypted(armor),
            (None, Some(plain)) => KeyMaterial::Plain(plain),
            (Some(_), Some(_)) => {
                return Err("journal identity file has both encrypted and plaintext key material");
            }
            (None, None) => return Err("journal identity file has no key material"),
        };
        Ok(Self {
            device_name: wire.device_name,
            key,
        })
    }
}

/// This device's stored identity label and whether it is passphrase-protected,
/// without decrypting anything. `None` when no identity file exists here.
pub fn device_identity_info(paths: &KeyPaths) -> Result<Option<DeviceIdentityInfo>> {
    if !paths.identity_file.exists() {
        return Ok(None);
    }
    let stored = read_stored_identity(&paths.identity_file)?;
    Ok(Some(DeviceIdentityInfo {
        name: stored.device_name,
        passphrase_protected: matches!(stored.key, KeyMaterial::Encrypted(_)),
    }))
}

/// Load this device's identity so encrypted entries can be read and written.
/// `passphrase` must be `Some` for a passphrase-protected identity and is
/// ignored for a plaintext one.
pub fn unlock_identity(
    paths: &KeyPaths,
    passphrase: Option<&SecretString>,
) -> Result<UnlockedIdentity> {
    let unlocked = decrypt_identity(paths, passphrase)?;

    // Validate via a self round-trip (encrypt to our own public key, decrypt with
    // the identity). Unlike checking against the shared roster, this holds even
    // before this device has been approved as a store recipient.
    let recipient = unlocked.recipient();
    let probe = cipher::PlaintextBytes::copy_from_slice(b"notema identity check");
    let encrypted = cipher::encrypt_to_recipients(std::slice::from_ref(&recipient), &probe)?;
    if cipher::decrypt_bytes_with_identity(&encrypted, &unlocked.identity)?.as_bytes()
        != probe.as_bytes()
    {
        return Err(EncryptionError::IdentityCheckFailed);
    }

    // Trust-on-first-use / advance the roster pins now that we're in at rest, so a
    // later rollback of anything seen up to now is detectable.
    recipients::refresh_trust_pins(paths);

    Ok(unlocked)
}

/// Reject an empty passphrase before it wraps any key material. An empty string
/// would silently degrade to plaintext-equivalent scrypt, so both write paths
/// route through this guard.
fn reject_empty_passphrase(passphrase: Option<&SecretString>) -> Result<()> {
    if matches!(passphrase, Some(passphrase) if passphrase.expose_secret().is_empty()) {
        return Err(EncryptionError::EmptyPassphrase);
    }
    Ok(())
}

/// Re-wrap this device's stored identity with a different passphrase state:
/// `current` unlocks it as stored now, `new` chooses how to store it going
/// forward (`Some` = scrypt-wrapped, `None` = plaintext mode-0600). Only rewrites
/// the local identity file; the keypair and all entries are untouched.
pub fn set_identity_passphrase(
    paths: &KeyPaths,
    current: Option<&SecretString>,
    new: Option<&SecretString>,
) -> Result<()> {
    reject_empty_passphrase(new)?;
    let stored = read_stored_identity(&paths.identity_file)?;
    let identity = decrypt_identity(paths, current)?;
    write_stored_identity(paths, &stored.device_name, &identity, new)
}

/// Read this device's identity file verbatim, for snapshotting before a rotation
/// so it can be put back byte-for-byte if the rotation fails.
pub fn read_identity_file_bytes(paths: &KeyPaths) -> Result<Vec<u8>> {
    Ok(fs::read(&paths.identity_file)?)
}

/// Restore this device's identity file from bytes captured by
/// [`read_identity_file_bytes`], preserving the private-file mode (0600).
pub fn restore_identity_file(paths: &KeyPaths, bytes: &[u8]) -> Result<()> {
    write_private_file(&paths.identity_file, bytes)
}

/// Generate this device's keypair and its public [`Recipient`], writing the
/// private identity (scrypt-wrapped when `passphrase` is `Some`, plaintext
/// mode-0600 otherwise). Shared by the store-creating and the joining device.
pub(crate) fn create_device_identity(
    paths: &KeyPaths,
    name: &str,
    passphrase: Option<&SecretString>,
) -> Result<(Recipient, UnlockedIdentity)> {
    if name.trim().is_empty() {
        return Err(EncryptionError::EmptyDeviceName);
    }

    let identity = UnlockedIdentity {
        identity: x25519::Identity::generate(),
        signing: generate_signing_key()?,
    };
    let recipient = Recipient {
        name: name.to_string(),
        enc_key: identity.public_key(),
        sign_key: identity.signing_public(),
    };
    write_stored_identity(paths, name, &identity, passphrase)?;
    Ok((recipient, identity))
}

/// Write this device's identity file, scrypt-wrapping the private key material
/// when a passphrase is given and storing it plaintext (mode 0600) otherwise.
/// Both the age key and the Ed25519 signing seed are bundled together so the same
/// passphrase choice protects both.
pub(crate) fn write_stored_identity(
    paths: &KeyPaths,
    name: &str,
    identity: &UnlockedIdentity,
    passphrase: Option<&SecretString>,
) -> Result<()> {
    reject_empty_passphrase(passphrase)?;
    let bundle = SecretBundle {
        x25519: Zeroizing::new(identity.identity.to_string().expose_secret().to_string()),
        ed25519: Zeroizing::new(hex::encode(identity.signing.to_bytes())),
    };
    let bundle_toml = Zeroizing::new(toml::to_string(&bundle)?);
    let (encrypted_keys, plain_keys) = match passphrase {
        Some(passphrase) => (
            Some(encrypt_secret(bundle_toml.as_bytes(), passphrase)?),
            None,
        ),
        None => (None, Some(bundle_toml.clone())),
    };
    let stored = StoredIdentityWire {
        device_name: name.to_string(),
        encrypted_keys,
        plain_keys,
    };
    // The serialized document carries the plaintext key bundle in the
    // no-passphrase case; zeroize the buffer once it's on disk.
    let serialized = Zeroizing::new(toml::to_string_pretty(&stored)?);
    write_private_file(&paths.identity_file, serialized.as_bytes())
}

fn decrypt_identity(
    paths: &KeyPaths,
    passphrase: Option<&SecretString>,
) -> Result<UnlockedIdentity> {
    let stored = read_stored_identity(&paths.identity_file)?;
    // The decrypted secret bundle lives in this string; zeroize it on drop so it
    // doesn't linger in freed heap after we parse it into keys.
    let bundle_toml: Zeroizing<String> = match &stored.key {
        KeyMaterial::Encrypted(armor) => {
            let passphrase = passphrase.ok_or(EncryptionError::PassphraseRequired)?;
            let identity = age::scrypt::Identity::new(passphrase.clone());
            Zeroizing::new(String::from_utf8(age::decrypt(
                &identity,
                armor.as_bytes(),
            )?)?)
        }
        KeyMaterial::Plain(plain) => plain.clone(),
    };
    let bundle: SecretBundle = toml::from_str(&bundle_toml)?;
    let identity = x25519::Identity::from_str(bundle.x25519.trim())
        .map_err(|_| EncryptionError::MalformedStoredIdentity)?;
    let seed_bytes = Zeroizing::new(hex::decode(bundle.ed25519.trim())?);
    let seed = <[u8; 32]>::try_from(seed_bytes.as_slice())
        .map_err(|_| EncryptionError::MalformedStoredIdentity)?;
    Ok(UnlockedIdentity {
        identity,
        signing: SigningKey::from_bytes(&seed),
    })
}

fn read_stored_identity(path: &Path) -> Result<StoredIdentity> {
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

fn encrypt_secret(plaintext: &[u8], passphrase: &SecretString) -> Result<Zeroizing<String>> {
    let recipient = age::scrypt::Recipient::new(passphrase.clone());
    Ok(Zeroizing::new(age::encrypt_and_armor(
        &recipient, plaintext,
    )?))
}
