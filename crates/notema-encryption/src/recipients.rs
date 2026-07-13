use crate::identity::{UnlockedIdentity, create_device_identity, write_stored_identity};
use crate::signing::{generate_signing_key, parse_signing_public, sign_bytes};
use crate::{EncryptionError, KeyPaths, Result, roster};
use age::secrecy::SecretString;
use age::x25519;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// A device that entries are encrypted to: its age public key, its Ed25519
/// signing key (which authorizes and is authorized by roster ops), and the
/// human-facing name that identifies the device in
/// `notema encryption device list`. Recorded in the signed `devices.toml` roster
/// and in pending requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipient {
    pub name: String,
    /// The age (X25519) recipient public key, `age1…`.
    pub encryption_key: String,
    /// The device's signing public key, `ed25519:<hex>`.
    pub signing_key: String,
}

impl Recipient {
    /// A short, human-comparable fingerprint of this device covering both its
    /// encryption and signing keys — shown at approval time for an out-of-band
    /// check against what the joining device displays.
    pub fn fingerprint(&self) -> String {
        roster::fingerprint(&self.encryption_key, &self.signing_key)
    }
}

/// Generate this device's keypair and write its private identity, then seed the
/// signed roster with a self-signed genesis op naming this device and pin it
/// locally. Used by the device that creates the store.
pub fn initialize_store_identity(
    paths: &KeyPaths,
    name: &str,
    passphrase: Option<&SecretString>,
) -> Result<Recipient> {
    if paths.has_roster() {
        return Err(EncryptionError::RosterExists);
    }
    let (recipient, identity) = create_device_identity(paths, name, passphrase)?;
    append_op(paths, &identity, roster::OpKind::Genesis, &recipient)?;
    advance_trust_pins(paths)?;
    Ok(recipient)
}

/// The store's current recipients, obtained by **verifying** the signed roster
/// against this device's local trust pins. Empty when the store isn't encrypted.
/// Returns [`EncryptionError::RosterUnverified`](crate::EncryptionError) — and so
/// refuses to hand back any recipient set — if the roster was tampered with or
/// rolled back.
pub fn read_recipients(paths: &KeyPaths) -> Result<Vec<Recipient>> {
    if !paths.devices_file.exists() {
        return Ok(Vec::new());
    }
    Ok(verified_roster(paths)?.recipients)
}

/// Verify the signed roster against the local pins, returning the current
/// recipient set plus the genesis/head hashes to pin.
fn verified_roster(paths: &KeyPaths) -> Result<roster::Verified> {
    let ops = roster::read_ops(&paths.devices_file)?;
    let pins = roster::read_pins(&paths.trust_file)?;
    roster::verify(&ops, &pins)
}

/// Advance this device's trust pins to the roster's current, verified head (also
/// recording the genesis on first sight). Called after a change this device made
/// or observed, so a later rollback below this point is detectable.
pub fn advance_trust_pins(paths: &KeyPaths) -> Result<()> {
    let verified = verified_roster(paths)?;
    roster::write_pins(
        &paths.trust_file,
        &verified.genesis_hash,
        &verified.head_hash,
    )
}

/// Best-effort pin refresh: pins the genesis+head on first valid sight (trust on
/// first use, e.g. a freshly joined device) and advances the head afterwards.
/// Silently does nothing when the store isn't encrypted or the roster doesn't
/// verify — the failing read path is where tampering is surfaced.
pub(crate) fn refresh_trust_pins(paths: &KeyPaths) {
    if paths.devices_file.exists()
        && let Ok(verified) = verified_roster(paths)
    {
        let _ = roster::write_pins(
            &paths.trust_file,
            &verified.genesis_hash,
            &verified.head_hash,
        );
    }
}

/// Append a signed `add` op naming `recipient`, authorized by `signer` (which
/// must already be a trusted recipient). Rejects a key or name that already
/// exists so a device can't be added twice or shadow another's label.
pub fn add_recipient(
    paths: &KeyPaths,
    signer: &UnlockedIdentity,
    recipient: &Recipient,
) -> Result<()> {
    validate_recipient(recipient)?;
    let recipients = read_recipients(paths)?;
    if recipients.iter().any(|r| r.encryption_key == recipient.encryption_key) {
        return Err(EncryptionError::RecipientExists {
            name: recipient.name.clone(),
        });
    }
    if recipients.iter().any(|r| r.name == recipient.name) {
        return Err(EncryptionError::RecipientNameTaken {
            name: recipient.name.clone(),
        });
    }
    append_op(paths, signer, roster::OpKind::Add, recipient)
}

/// Append a signed `revoke` op for the recipient named `name`, authorized by
/// `signer`. Refuses to revoke the last recipient, which would leave the store
/// impossible to re-encrypt.
pub fn revoke_recipient(paths: &KeyPaths, signer: &UnlockedIdentity, name: &str) -> Result<()> {
    let recipients = read_recipients(paths)?;
    let Some(target) = recipients.iter().find(|r| r.name == name) else {
        return Err(EncryptionError::UnknownRecipient {
            name: name.to_string(),
        });
    };
    if recipients.len() == 1 {
        return Err(EncryptionError::LastRecipient);
    }
    append_op(paths, signer, roster::OpKind::Revoke, target)
}

/// Append a signed `rename` op relabelling a recipient, authorized by `signer`.
/// No re-encryption needed — the keys don't change.
pub fn rename_recipient(
    paths: &KeyPaths,
    signer: &UnlockedIdentity,
    old: &str,
    new: &str,
) -> Result<()> {
    if new.trim().is_empty() {
        return Err(EncryptionError::EmptyRecipientName);
    }
    let recipients = read_recipients(paths)?;
    if recipients.iter().any(|recipient| recipient.name == new) {
        return Err(EncryptionError::RecipientNameTaken {
            name: new.to_string(),
        });
    }
    let Some(target) = recipients.iter().find(|recipient| recipient.name == old) else {
        return Err(EncryptionError::UnknownRecipient {
            name: old.to_string(),
        });
    };
    let relabelled = Recipient {
        name: new.to_string(),
        encryption_key: target.encryption_key.clone(),
        signing_key: target.signing_key.clone(),
    };
    append_op(paths, signer, roster::OpKind::Rename, &relabelled)
}

/// Whether `identity`'s public key is one of the store's current recipients —
/// i.e. this device can already decrypt the store, and so is allowed to
/// re-encrypt it (and sign roster ops) when approving or removing another device.
pub fn identity_is_recipient(paths: &KeyPaths, identity: &UnlockedIdentity) -> Result<bool> {
    let own = identity.public_key();
    Ok(read_recipients(paths)?
        .iter()
        .any(|recipient| recipient.encryption_key == own))
}

/// Generate a fresh age *and* signing keypair for this device and append a signed
/// `add` op for it (authorized by the current, still-trusted key) so it joins the
/// roster *alongside* the old key under the same name. Returns the new public
/// [`Recipient`] and its not-yet-persisted unlocked identity. The old key stays a
/// recipient until [`drop_old_recipient`], so re-encryption during a rotation
/// can't lock this device out mid-way.
pub fn rotate_add_new_key(
    paths: &KeyPaths,
    old: &UnlockedIdentity,
) -> Result<(Recipient, UnlockedIdentity)> {
    let old_key = old.public_key();
    let recipients = read_recipients(paths)?;
    let Some(existing) = recipients
        .iter()
        .find(|recipient| recipient.encryption_key == old_key)
    else {
        return Err(EncryptionError::NotARecipient);
    };

    let new_identity = UnlockedIdentity {
        identity: x25519::Identity::generate(),
        signing: generate_signing_key()?,
    };
    let recipient = Recipient {
        name: existing.name.clone(),
        encryption_key: new_identity.public_key(),
        signing_key: new_identity.signing_public(),
    };
    // Signed by the old key, which is trusted until it's dropped below.
    append_op(paths, old, roster::OpKind::Add, &recipient)?;
    Ok((recipient, new_identity))
}

/// Persist the rotated identity as this device's key file, preserving its
/// passphrase state (`passphrase` re-wraps it, `None` stores it plaintext).
pub fn commit_rotated_identity(
    paths: &KeyPaths,
    recipient: &Recipient,
    identity: &UnlockedIdentity,
    passphrase: Option<&SecretString>,
) -> Result<()> {
    write_stored_identity(paths, &recipient.name, identity, passphrase)
}

/// Append a signed `revoke` op retiring the old key (the final step of a
/// rotation, after every entry has been re-encrypted to the new key). Authorized
/// by `signer` — the freshly rotated identity, which is now a trusted recipient.
pub fn drop_old_recipient(
    paths: &KeyPaths,
    signer: &UnlockedIdentity,
    old_key: &str,
) -> Result<()> {
    let recipients = read_recipients(paths)?;
    let Some(target) = recipients
        .iter()
        .find(|recipient| recipient.encryption_key == old_key)
    else {
        return Ok(());
    };
    append_op(paths, signer, roster::OpKind::Revoke, target)
}

/// Validate that a recipient carries a well-formed age recipient and Ed25519
/// signing key before it's admitted to the roster.
fn validate_recipient(recipient: &Recipient) -> Result<()> {
    if x25519::Recipient::from_str(&recipient.encryption_key).is_err() {
        return Err(EncryptionError::InvalidRecipientKey {
            key: recipient.encryption_key.clone(),
        });
    }
    if parse_signing_public(&recipient.signing_key).is_none() {
        return Err(EncryptionError::InvalidSigningKey {
            key: recipient.signing_key.clone(),
        });
    }
    Ok(())
}

/// Append a signed roster op describing `subject`, authorized by `signer` (whose
/// signing key must already be trusted for a non-genesis op).
fn append_op(
    paths: &KeyPaths,
    signer: &UnlockedIdentity,
    kind: roster::OpKind,
    subject: &Recipient,
) -> Result<()> {
    let signer_pub = signer.signing_public();
    roster::append(
        &paths.devices_file,
        kind,
        &subject.name,
        &subject.encryption_key,
        &subject.signing_key,
        &signer_pub,
        |bytes| sign_bytes(&signer.signing, bytes),
    )?;
    Ok(())
}
