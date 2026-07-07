//! Journal's encryption layer: per-device age keypairs, a signed append-only
//! device roster, passphrase-wrapped identities, and the helpers that turn
//! journal bytes into age ciphertext and back.
//!
//! It owns all of the app's cryptography and knows nothing about how entries or
//! assets are laid out on disk: it works on a [`KeyPaths`] and byte buffers, and
//! the storage layer decides which files those bytes belong to.
//!
//! Scope: this layer provides **confidentiality** (and, through the roster,
//! authenticated device membership) but **not** per-entry authenticity or author
//! attribution — entries and assets are encrypted, not signed. See the roster
//! module's "Residual threats" notes.

mod roster;

pub use age::secrecy::{ExposeSecret, SecretString};
use age::x25519;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error;
use zeroize::Zeroizing;

/// A unique hidden sibling temp path next to `target`, for atomic
/// write-then-rename. Named `.journal-<pid>-<rand>.<suffix>` in the target's
/// directory so it lands on the same filesystem as the eventual rename target.
pub(crate) fn sibling_temp_path(target: &Path, suffix: &str) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let mut noise = [0u8; 8];
    let _ = getrandom::getrandom(&mut noise);
    parent.join(format!(
        ".journal-{}-{}.{suffix}",
        std::process::id(),
        hex::encode(noise),
    ))
}

pub type Result<T> = std::result::Result<T, EncryptionError>;

/// The first three variants are meaningful to callers (prompt for a passphrase,
/// refuse to continue); the rest wrap an underlying failure typed rather than
/// boxed.
#[derive(Debug, Error)]
pub enum EncryptionError {
    /// An encrypted item was accessed without an unlocked identity. `context`
    /// names what was being read (e.g. `"entry"`, `"asset"`, `"store"`).
    #[error("encrypted {context} requires an unlocked journal encryption identity")]
    Locked { context: &'static str },

    /// Encrypted entries exist but the signed device roster needed to encrypt
    /// more is gone — continuing could leave the store partially encrypted.
    #[error(
        "encrypted entries already exist but the device roster is missing at {}; cannot safely continue encryption",
        .path.display()
    )]
    RecipientsMissing { path: PathBuf },

    /// The signed device roster failed verification: a forged/unauthorized op, a
    /// broken signature chain, a changed genesis, or a rolled-back history. The
    /// store refuses to encrypt or decrypt to an untrusted recipient set rather
    /// than silently trusting the tampered file. `detail` explains which check
    /// failed.
    #[error("device roster failed verification: {detail}")]
    RosterUnverified { detail: String },

    /// A precondition or input the encryption layer rejected: a duplicate or
    /// unknown recipient, an empty name or passphrase, malformed key material, or
    /// a missing passphrase. Carries a human-facing message.
    #[error("{0}")]
    Invalid(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("age encryption failed: {0}")]
    Encrypt(#[from] age::EncryptError),

    #[error("age decryption failed: {0}")]
    Decrypt(#[from] age::DecryptError),

    #[error("malformed encryption metadata: {0}")]
    TomlRead(#[from] toml::de::Error),

    #[error("could not serialize encryption metadata: {0}")]
    TomlWrite(#[from] toml::ser::Error),

    #[error("invalid hex encoding: {0}")]
    Hex(#[from] hex::FromHexError),

    #[error("invalid UTF-8 in key material: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

impl From<String> for EncryptionError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}

impl From<&str> for EncryptionError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.to_string())
    }
}

/// A device that entries are encrypted to: its age public key, its Ed25519
/// signing key (which authorizes and is authorized by roster ops), and the
/// human-facing name that identifies the device in
/// `journal encryption device list`. Recorded in the signed `devices.toml` roster
/// and in pending requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipient {
    pub name: String,
    /// The age (X25519) recipient public key, `age1…`.
    pub key: String,
    /// The device's signing public key, `ed25519:<hex>`.
    pub sign: String,
}

impl Recipient {
    /// A short, human-comparable fingerprint of this device covering both its
    /// encryption and signing keys — shown at approval time for an out-of-band
    /// check against what the joining device displays.
    pub fn fingerprint(&self) -> String {
        roster::fingerprint(&self.key, &self.sign)
    }
}

/// A join request waiting to be approved: the requesting device's [`Recipient`]
/// and the stable `id` derived from its key (the `pending-<id>.toml` file name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub id: String,
    pub recipient: Recipient,
}

/// The non-secret facts about this device's stored identity, readable without a
/// passphrase: how it labels itself and whether unlocking needs a passphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentityInfo {
    pub name: String,
    pub passphrase_protected: bool,
}

/// The on-disk `pending-<id>.toml` document: the requesting device's
/// [`Recipient`] plus a self-signature over it, proving the request came from a
/// holder of that signing key (a corrupt or forged request is dropped on read).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingFile {
    recipient: Recipient,
    sig: String,
}

#[derive(Serialize)]
struct PendingFileRef<'a> {
    recipient: &'a Recipient,
    sig: &'a str,
}

/// The secret key material for a device, serialized inside the (optionally
/// scrypt-wrapped) `identity.age`: the age private key and the Ed25519 signing
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
    /// scrypt-wrapped bundle, opened with a passphrase.
    Encrypted(Vec<u8>),
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

/// The on-disk shape of `identity.age`: `device_name` plus exactly one of the
/// scrypt-wrapped or plaintext key fields. Kept as the wire form so the file
/// layout is unchanged; [`StoredIdentity`] is the validated in-memory view.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredIdentityWire {
    device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encrypted_identity: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plain_identity: Option<Zeroizing<String>>,
}

impl TryFrom<StoredIdentityWire> for StoredIdentity {
    type Error = &'static str;

    fn try_from(wire: StoredIdentityWire) -> std::result::Result<Self, Self::Error> {
        let key = match (wire.encrypted_identity, wire.plain_identity) {
            (Some(blob), None) => KeyMaterial::Encrypted(blob),
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

#[derive(Clone)]
pub struct UnlockedIdentity {
    identity: x25519::Identity,
    signing: SigningKey,
}

impl UnlockedIdentity {
    fn recipient(&self) -> x25519::Recipient {
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

/// The file locations of a store's key material — everything the encryption
/// layer reads or writes, and nothing about the journal's entries. Public key
/// material lives in the synced `<root>/.age/` folder; the private identity and
/// the roster trust pins live next to the config and are never synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPaths {
    /// The hidden, synced key folder holding the signed `devices.toml` roster and
    /// any `pending-<id>.toml` join requests.
    pub age_dir: PathBuf,
    /// The signed, append-only device roster (`<root>/.age/devices.toml`).
    pub devices_file: PathBuf,
    /// This device's private age identity (`identity.age`), never synced.
    pub identity_file: PathBuf,
    /// This device's local trust pins for the roster (genesis + last-seen head).
    /// Sits next to the identity, never synced, so a sync-folder attacker can't
    /// reach it.
    pub trust_file: PathBuf,
}

impl KeyPaths {
    /// Derive the key locations from the journal root and the config directory.
    pub fn new(journal_root: impl AsRef<Path>, config_dir: impl AsRef<Path>) -> Self {
        let age_dir = journal_root.as_ref().join(".age");
        let config_dir = config_dir.as_ref();
        Self {
            devices_file: age_dir.join("devices.toml"),
            identity_file: config_dir.join("identity.age"),
            trust_file: config_dir.join("devices-trust.toml"),
            age_dir,
        }
    }

    /// Like [`new`](Self::new), taking the config *file* and reading its parent
    /// directory for the identity location.
    pub fn for_config(config_path: &Path, journal_root: &Path) -> Result<Self> {
        let config_dir = config_path
            .parent()
            .ok_or("config path has no parent directory")?;
        Ok(Self::new(journal_root, config_dir))
    }
}

pub fn has_devices_file(paths: &KeyPaths) -> bool {
    paths.devices_file.exists()
}

pub fn has_identity_file(paths: &KeyPaths) -> bool {
    paths.identity_file.exists()
}

/// The store's current recipients, obtained by **verifying** the signed roster
/// against this device's local trust pins. Empty when the store isn't encrypted.
/// Returns [`EncryptionError::RosterUnverified`] — and so refuses to hand back
/// any recipient set — if the roster was tampered with or rolled back.
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
    roster::write_pins(&paths.trust_file, &verified.genesis, &verified.head)
}

/// Best-effort pin refresh: pins the genesis+head on first valid sight (trust on
/// first use, e.g. a freshly joined device) and advances the head afterwards.
/// Silently does nothing when the store isn't encrypted or the roster doesn't
/// verify — the failing read path is where tampering is surfaced.
fn refresh_trust_pins(paths: &KeyPaths) {
    if paths.devices_file.exists()
        && let Ok(verified) = verified_roster(paths)
    {
        let _ = roster::write_pins(&paths.trust_file, &verified.genesis, &verified.head);
    }
}

/// The first recipient's public key, for display (e.g. after `journal encryption enable`).
pub fn public_recipient(paths: &KeyPaths) -> Result<String> {
    read_recipients(paths)?
        .into_iter()
        .next()
        .map(|recipient| recipient.key)
        .ok_or_else(|| "journal encryption recipients file is empty".into())
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

/// Generate this device's keypair and write its private identity (scrypt-wrapped
/// when `passphrase` is `Some`, plaintext mode-0600 otherwise), then seed the
/// signed roster with a self-signed genesis op naming this device and pin it
/// locally. Used by the device that creates the store.
pub fn initialize_store_identity(
    paths: &KeyPaths,
    name: &str,
    passphrase: Option<&SecretString>,
) -> Result<Recipient> {
    if has_devices_file(paths) {
        return Err("device roster already exists; use request_store_access to join instead".into());
    }
    let (recipient, identity) = create_device_identity(paths, name, passphrase)?;
    append_op(paths, &identity, roster::OpKind::Genesis, &recipient)?;
    advance_trust_pins(paths)?;
    Ok(recipient)
}

/// Generate this device's keypair for a store that already exists elsewhere, and
/// drop a self-signed `pending-<id>.toml` join request into the shared `.age/`
/// folder. Does not touch the roster — a device that can decrypt approves the
/// request by appending a signed `add` op.
pub fn request_store_access(
    paths: &KeyPaths,
    name: &str,
    passphrase: Option<&SecretString>,
) -> Result<Recipient> {
    let (recipient, identity) = create_device_identity(paths, name, passphrase)?;
    write_pending(paths, &recipient, &identity)?;
    Ok(recipient)
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
    let probe = b"journal identity check";
    let encrypted = encrypt_to_recipients(std::slice::from_ref(&recipient), probe)?;
    if decrypt_bytes_with_identity(&encrypted, &unlocked.identity)? != probe {
        return Err("journal encryption identity check failed".into());
    }

    // Trust-on-first-use / advance the roster pins now that we're in at rest, so a
    // later rollback of anything seen up to now is detectable.
    refresh_trust_pins(paths);

    Ok(unlocked)
}

pub fn encrypt_to_file(
    paths: &KeyPaths,
    plaintext: &[u8],
    output: &Path,
) -> Result<()> {
    fs::write(output, encrypt_bytes(paths, plaintext)?)?;
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
    let recipients = recipient_keys(&read_recipients(paths)?)?;
    encrypt_to_recipients(&recipients, plaintext)
}

/// Encrypt a freshly created entry to every store recipient, plus this device's
/// own key when unlocked — so the authoring device can always re-read what it
/// wrote, even a joining device whose key isn't yet an approved recipient.
pub fn encrypt_new_entry(
    paths: &KeyPaths,
    plaintext: &[u8],
    identity: Option<&UnlockedIdentity>,
) -> Result<Vec<u8>> {
    let mut recipients = recipient_keys(&read_recipients(paths)?)?;
    if let Some(identity) = identity {
        let own = identity.recipient();
        if !recipients.iter().any(|r| r.to_string() == own.to_string()) {
            recipients.push(own);
        }
    }
    encrypt_to_recipients(&recipients, plaintext)
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
    if recipients.iter().any(|r| r.key == recipient.key) {
        return Err(format!("recipient '{}' is already present", recipient.name).into());
    }
    if recipients.iter().any(|r| r.name == recipient.name) {
        return Err(format!(
            "a recipient named '{}' already exists; pick a unique name",
            recipient.name
        )
        .into());
    }
    append_op(paths, signer, roster::OpKind::Add, recipient)
}

/// Append a signed `remove` op for the recipient named `name`, authorized by
/// `signer`. Refuses to remove the last recipient, which would leave the store
/// impossible to re-encrypt.
pub fn remove_recipient(
    paths: &KeyPaths,
    signer: &UnlockedIdentity,
    name: &str,
) -> Result<()> {
    let recipients = read_recipients(paths)?;
    let Some(target) = recipients.iter().find(|r| r.name == name) else {
        return Err(format!("no recipient named '{name}'").into());
    };
    if recipients.len() == 1 {
        return Err("cannot remove the last recipient; the store would become unreadable".into());
    }
    append_op(paths, signer, roster::OpKind::Remove, target)
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
        return Err("new recipient name cannot be empty".into());
    }
    let recipients = read_recipients(paths)?;
    if recipients.iter().any(|recipient| recipient.name == new) {
        return Err(format!("a recipient named '{new}' already exists").into());
    }
    let Some(target) = recipients.iter().find(|recipient| recipient.name == old) else {
        return Err(format!("no recipient named '{old}'").into());
    };
    let relabelled = Recipient {
        name: new.to_string(),
        key: target.key.clone(),
        sign: target.sign.clone(),
    };
    append_op(paths, signer, roster::OpKind::Rename, &relabelled)
}

/// Whether `identity`'s public key is one of the store's current recipients —
/// i.e. this device can already decrypt the store, and so is allowed to
/// re-encrypt it (and sign roster ops) when approving or removing another device.
pub fn identity_is_recipient(
    paths: &KeyPaths,
    identity: &UnlockedIdentity,
) -> Result<bool> {
    let own = identity.public_key();
    Ok(read_recipients(paths)?
        .iter()
        .any(|recipient| recipient.key == own))
}

/// The pending join requests in the shared `.age/` folder, sorted by name.
pub fn read_pending(paths: &KeyPaths) -> Result<Vec<PendingRequest>> {
    let mut requests = Vec::new();
    if !paths.age_dir.exists() {
        return Ok(requests);
    }
    for entry in fs::read_dir(&paths.age_dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(id) = name
            .strip_prefix("pending-")
            .and_then(|rest| rest.strip_suffix(".toml"))
        else {
            continue;
        };
        let parsed: PendingFile = toml::from_str(&fs::read_to_string(entry.path())?)?;
        // Drop a request whose self-signature doesn't check out: it was corrupted
        // or forged in the synced folder. A genuine device can re-submit.
        if !verify_signature(
            &parsed.recipient.sign,
            &pending_signing_bytes(&parsed.recipient),
            &parsed.sig,
        ) {
            continue;
        }
        requests.push(PendingRequest {
            id: id.to_string(),
            recipient: parsed.recipient,
        });
    }
    requests.sort_by(|a, b| a.recipient.name.cmp(&b.recipient.name));
    Ok(requests)
}

/// Delete a processed join request. A no-op if it's already gone.
pub fn remove_pending(paths: &KeyPaths, id: &str) -> Result<()> {
    let path = paths.age_dir.join(format!("pending-{id}.toml"));
    if path.exists() {
        fs::remove_file(path)?;
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
    if matches!(new, Some(passphrase) if passphrase.expose_secret().is_empty()) {
        return Err("encryption passphrase cannot be empty".into());
    }
    let stored = read_stored_identity(&paths.identity_file)?;
    let identity = decrypt_identity(paths, current)?;
    write_stored_identity(paths, &stored.device_name, &identity, new)
}

/// Build the byte buffer a device self-signs in its join request, binding its
/// name and both public keys under a distinct domain so it can't be replayed as
/// any other signature.
fn pending_signing_bytes(recipient: &Recipient) -> Vec<u8> {
    let mut buf = Vec::new();
    roster::push_field(&mut buf, b"journal.pending.v1");
    roster::push_field(&mut buf, recipient.name.as_bytes());
    roster::push_field(&mut buf, recipient.key.as_bytes());
    roster::push_field(&mut buf, recipient.sign.as_bytes());
    buf
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
    let Some(existing) = recipients.iter().find(|recipient| recipient.key == old_key) else {
        return Err("this device is not a current recipient; cannot rotate".into());
    };

    let new_identity = UnlockedIdentity {
        identity: x25519::Identity::generate(),
        signing: generate_signing_key()?,
    };
    let recipient = Recipient {
        name: existing.name.clone(),
        key: new_identity.public_key(),
        sign: new_identity.signing_public(),
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

/// Append a signed `remove` op retiring the old key (the final step of a
/// rotation, after every entry has been re-encrypted to the new key). Authorized
/// by `signer` — the freshly rotated identity, which is now a trusted recipient.
pub fn drop_old_recipient(
    paths: &KeyPaths,
    signer: &UnlockedIdentity,
    old_key: &str,
) -> Result<()> {
    let recipients = read_recipients(paths)?;
    let Some(target) = recipients.iter().find(|recipient| recipient.key == old_key) else {
        return Ok(());
    };
    append_op(paths, signer, roster::OpKind::Remove, target)
}

fn create_device_identity(
    paths: &KeyPaths,
    name: &str,
    passphrase: Option<&SecretString>,
) -> Result<(Recipient, UnlockedIdentity)> {
    if name.trim().is_empty() {
        return Err("device name cannot be empty".into());
    }

    let identity = UnlockedIdentity {
        identity: x25519::Identity::generate(),
        signing: generate_signing_key()?,
    };
    let recipient = Recipient {
        name: name.to_string(),
        key: identity.public_key(),
        sign: identity.signing_public(),
    };
    write_stored_identity(paths, name, &identity, passphrase)?;
    Ok((recipient, identity))
}

/// Validate that a recipient carries a well-formed age recipient and Ed25519
/// signing key before it's admitted to the roster.
fn validate_recipient(recipient: &Recipient) -> Result<()> {
    if x25519::Recipient::from_str(&recipient.key).is_err() {
        return Err(format!("'{}' is not a valid age recipient", recipient.key).into());
    }
    if parse_signing_public(&recipient.sign).is_none() {
        return Err(format!("'{}' is not a valid signing key", recipient.sign).into());
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
        &subject.key,
        &subject.sign,
        &signer_pub,
        |bytes| sign_bytes(&signer.signing, bytes),
    )?;
    Ok(())
}

/// Write this device's identity file, scrypt-wrapping the private key material
/// when a passphrase is given and storing it plaintext (mode 0600) otherwise.
/// Both the age key and the Ed25519 signing seed are bundled together so the same
/// passphrase choice protects both.
fn write_stored_identity(
    paths: &KeyPaths,
    name: &str,
    identity: &UnlockedIdentity,
    passphrase: Option<&SecretString>,
) -> Result<()> {
    if matches!(passphrase, Some(passphrase) if passphrase.expose_secret().is_empty()) {
        return Err("encryption passphrase cannot be empty".into());
    }
    let bundle = SecretBundle {
        x25519: Zeroizing::new(identity.identity.to_string().expose_secret().to_string()),
        ed25519: Zeroizing::new(hex::encode(identity.signing.to_bytes())),
    };
    let bundle_toml = Zeroizing::new(toml::to_string(&bundle)?);
    let (encrypted_identity, plain_identity) = match passphrase {
        Some(passphrase) => (Some(encrypt_secret(bundle_toml.as_bytes(), passphrase)?), None),
        None => (None, Some(bundle_toml.clone())),
    };
    let stored = StoredIdentityWire {
        device_name: name.to_string(),
        encrypted_identity,
        plain_identity,
    };
    // The serialized document carries the plaintext key bundle in the
    // no-passphrase case; zeroize the buffer once it's on disk.
    let serialized = Zeroizing::new(toml::to_string_pretty(&stored)?);
    write_private_file(&paths.identity_file, serialized.as_bytes())
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

/// Generate a fresh Ed25519 signing keypair from OS randomness.
fn generate_signing_key() -> Result<SigningKey> {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(&mut seed[..])
        .map_err(|error| format!("failed to gather randomness for signing key: {error}"))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// A signing key's public half encoded as `ed25519:<hex>`.
fn signing_public(signing: &SigningKey) -> String {
    format!("ed25519:{}", hex::encode(signing.verifying_key().to_bytes()))
}

/// Sign `msg` with `signing`, returning the hex Ed25519 signature.
fn sign_bytes(signing: &SigningKey, msg: &[u8]) -> String {
    hex::encode(signing.sign(msg).to_bytes())
}

/// Parse an `ed25519:<hex>` public key into a verifier, or `None` if malformed.
fn parse_signing_public(signer: &str) -> Option<VerifyingKey> {
    let hex_key = signer.strip_prefix("ed25519:")?;
    let bytes = hex::decode(hex_key).ok()?;
    let array = <[u8; 32]>::try_from(bytes.as_slice()).ok()?;
    VerifyingKey::from_bytes(&array).ok()
}

/// Verify a hex Ed25519 signature by the `ed25519:<hex>` public key over `msg`.
/// Any malformed input verifies as `false` rather than erroring, so a corrupt
/// roster op is simply rejected. Uses strict verification (rejects non-canonical
/// signatures and small-order keys).
pub(crate) fn verify_signature(signer: &str, msg: &[u8], sig_hex: &str) -> bool {
    let Some(verifying) = parse_signing_public(signer) else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };
    let Ok(sig_array) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
        return false;
    };
    verifying
        .verify_strict(msg, &Signature::from_bytes(&sig_array))
        .is_ok()
}

fn recipient_keys(recipients: &[Recipient]) -> Result<Vec<x25519::Recipient>> {
    if recipients.is_empty() {
        return Err("journal encryption recipients file is empty".into());
    }
    recipients
        .iter()
        .map(|recipient| Ok(x25519::Recipient::from_str(&recipient.key)?))
        .collect()
}

fn encrypt_to_recipients(recipients: &[x25519::Recipient], plaintext: &[u8]) -> Result<Vec<u8>> {
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

fn decrypt_bytes_with_identity(
    ciphertext: &[u8],
    identity: &x25519::Identity,
) -> Result<Vec<u8>> {
    Ok(age::decrypt(identity, ciphertext)?)
}

fn encrypt_secret(plaintext: &[u8], passphrase: &SecretString) -> Result<Vec<u8>> {
    let recipient = age::scrypt::Recipient::new(passphrase.clone());
    Ok(age::encrypt(&recipient, plaintext)?)
}

fn decrypt_identity(
    paths: &KeyPaths,
    passphrase: Option<&SecretString>,
) -> Result<UnlockedIdentity> {
    let stored = read_stored_identity(&paths.identity_file)?;
    // The decrypted secret bundle lives in this string; zeroize it on drop so it
    // doesn't linger in freed heap after we parse it into keys.
    let bundle_toml: Zeroizing<String> = match &stored.key {
        KeyMaterial::Encrypted(blob) => {
            let passphrase = passphrase
                .ok_or("journal identity is passphrase-protected; a passphrase is required")?;
            let identity = age::scrypt::Identity::new(passphrase.clone());
            Zeroizing::new(String::from_utf8(age::decrypt(&identity, blob)?)?)
        }
        KeyMaterial::Plain(plain) => plain.clone(),
    };
    let bundle: SecretBundle = toml::from_str(&bundle_toml)?;
    let identity = x25519::Identity::from_str(bundle.x25519.trim())?;
    let seed_bytes = Zeroizing::new(hex::decode(bundle.ed25519.trim())?);
    let seed = <[u8; 32]>::try_from(seed_bytes.as_slice())
        .map_err(|_| "journal identity signing key has the wrong length")?;
    Ok(UnlockedIdentity {
        identity,
        signing: SigningKey::from_bytes(&seed),
    })
}

fn read_stored_identity(path: &Path) -> Result<StoredIdentity> {
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

fn write_pending(
    paths: &KeyPaths,
    recipient: &Recipient,
    identity: &UnlockedIdentity,
) -> Result<()> {
    fs::create_dir_all(&paths.age_dir)?;
    let sig = sign_bytes(&identity.signing, &pending_signing_bytes(recipient));
    let document = PendingFileRef {
        recipient,
        sig: &sig,
    };
    let path = paths.age_dir.join(pending_file_name(&recipient.key));
    atomic_write(&path, toml::to_string_pretty(&document)?.as_bytes())
}

/// Write `content` to `path` via a sibling temp file plus rename, so a crash
/// mid-write can't truncate an existing file (which would strand every device)
/// or leave a half-written join request behind.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let temp = crate::sibling_temp_path(path, "tmp");
    fs::write(&temp, content)?;
    fs::rename(&temp, path)?;
    Ok(())
}

/// The `pending-<id>.toml` file name for a recipient, where `<id>` is a stable,
/// filename-safe slice of the bech32 public key (unique enough to avoid
/// collisions between devices, deterministic so a re-run overwrites its own).
fn pending_file_name(key: &str) -> String {
    let id: String = key
        .strip_prefix("age1")
        .unwrap_or(key)
        .chars()
        .take(12)
        .collect();
    format!("pending-{id}.toml")
}

fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
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

        initialize_store_identity(&laptop, "laptop", Some(&SecretString::from("pw")))
            .unwrap();
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

        initialize_store_identity(&laptop, "laptop", Some(&SecretString::from("pw")))
            .unwrap();
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
            initialize_store_identity(&paths, "laptop", Some(&SecretString::from("pw")))
                .unwrap();
        let identity = unlock_identity(&paths, Some(&SecretString::from("pw"))).unwrap();

        // Same key → rejected for the key clash.
        assert!(add_recipient(&paths, &identity, &recipient).is_err());
        // Same name, different (valid) key → rejected for the name clash.
        let same_name_new_key = Recipient {
            key: "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsuaxjx".to_string(),
            ..recipient
        };
        assert!(add_recipient(&paths, &identity, &same_name_new_key).is_err());
    }

    #[test]
    fn remove_recipient_refuses_the_last_one() {
        let dir = tempdir().unwrap();
        let paths = paths_in(dir.path());
        initialize_store_identity(&paths, "laptop", Some(&SecretString::from("pw")))
            .unwrap();
        let identity = unlock_identity(&paths, Some(&SecretString::from("pw"))).unwrap();

        assert!(remove_recipient(&paths, &identity, "laptop").is_err());
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
    fn decrypt_file_bytes_from(
        identity: &UnlockedIdentity,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>> {
        decrypt_bytes_with_identity(ciphertext, &identity.identity)
    }
}
