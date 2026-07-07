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
use zeroize::Zeroizing;

/// A device that entries are encrypted to: its age public key plus the
/// human-facing name that identifies the device in
/// `journal encryption device list`. Recorded in `recipients.toml` and in pending requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipient {
    pub name: String,
    pub key: String,
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

/// The on-disk `recipients.toml` document: a list of `[[recipient]]` tables.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipientsFile {
    #[serde(default, rename = "recipient")]
    recipients: Vec<Recipient>,
}

/// Serialize-only view of the recipients document that borrows the slice, so
/// writing back doesn't clone every recipient.
#[derive(Serialize)]
struct RecipientsFileRef<'a> {
    #[serde(rename = "recipient")]
    recipient: &'a [Recipient],
}

/// The on-disk `pending-<id>.toml` document: a single `[recipient]` table.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingFile {
    recipient: Recipient,
}

#[derive(Serialize)]
struct PendingFileRef<'a> {
    recipient: &'a Recipient,
}

/// This device's private age identity plus the label it stores itself under.
/// Exactly one of `encrypted_identity` (scrypt-wrapped) / `plain_identity`
/// (stored in the clear, mode 0600) is present, per the device's passphrase
/// choice.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredIdentity {
    device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encrypted_identity: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plain_identity: Option<Zeroizing<String>>,
}

#[derive(Clone)]
pub struct UnlockedIdentity {
    identity: x25519::Identity,
}

impl UnlockedIdentity {
    fn recipient(&self) -> x25519::Recipient {
        self.identity.to_public()
    }

    /// This identity's public key string, for matching against recipients.
    pub fn public_key(&self) -> String {
        self.identity.to_public().to_string()
    }
}

pub fn has_recipients_file(paths: &JournalStorePaths) -> bool {
    paths.recipients_file.exists()
}

pub fn has_identity_file(paths: &JournalStorePaths) -> bool {
    paths.identity_file.exists()
}

/// The store's recipients, in file order. Empty when the store isn't encrypted.
pub fn read_recipients(paths: &JournalStorePaths) -> AppResult<Vec<Recipient>> {
    if !paths.recipients_file.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&paths.recipients_file)?;
    Ok(toml::from_str::<RecipientsFile>(&text)?.recipients)
}

/// The first recipient's public key, for display (e.g. after `journal encryption enable`).
pub fn public_recipient(paths: &JournalStorePaths) -> AppResult<String> {
    read_recipients(paths)?
        .into_iter()
        .next()
        .map(|recipient| recipient.key)
        .ok_or_else(|| "journal encryption recipients file is empty".into())
}

/// This device's stored identity label and whether it is passphrase-protected,
/// without decrypting anything. `None` when no identity file exists here.
pub fn device_identity_info(paths: &JournalStorePaths) -> AppResult<Option<DeviceIdentityInfo>> {
    if !paths.identity_file.exists() {
        return Ok(None);
    }
    let stored = read_stored_identity(&paths.identity_file)?;
    Ok(Some(DeviceIdentityInfo {
        name: stored.device_name,
        passphrase_protected: stored.encrypted_identity.is_some(),
    }))
}

/// Generate this device's keypair and write its private identity (scrypt-wrapped
/// when `passphrase` is `Some`, plaintext mode-0600 otherwise) as the store's
/// first recipient. Used by the device that creates the store.
pub fn initialize_store_identity(
    paths: &JournalStorePaths,
    name: &str,
    passphrase: Option<&SecretString>,
) -> AppResult<Recipient> {
    let recipient = create_device_identity(paths, name, passphrase)?;
    write_recipients(paths, std::slice::from_ref(&recipient))?;
    Ok(recipient)
}

/// Generate this device's keypair for a store that already exists elsewhere, and
/// drop a `pending-<id>.toml` join request into the shared `.age/` folder. Does
/// not touch `recipients.toml` — a device that can decrypt approves the request.
pub fn request_store_access(
    paths: &JournalStorePaths,
    name: &str,
    passphrase: Option<&SecretString>,
) -> AppResult<Recipient> {
    let recipient = create_device_identity(paths, name, passphrase)?;
    write_pending(paths, &recipient)?;
    Ok(recipient)
}

/// Load this device's identity so encrypted entries can be read and written.
/// `passphrase` must be `Some` for a passphrase-protected identity and is
/// ignored for a plaintext one.
pub fn unlock_identity(
    paths: &JournalStorePaths,
    passphrase: Option<&SecretString>,
) -> AppResult<UnlockedIdentity> {
    let identity = decrypt_identity(paths, passphrase)?;

    // Validate via a self round-trip (encrypt to our own public key, decrypt with
    // the identity). Unlike checking against the shared recipients file, this
    // holds even before this device has been approved as a store recipient.
    let recipient = identity.to_public();
    let probe = b"journal identity check";
    let encrypted = encrypt_to_recipients(std::slice::from_ref(&recipient), probe)?;
    if decrypt_bytes_with_identity(&encrypted, &identity)? != probe {
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

/// Encrypt bytes to every store recipient.
pub fn encrypt_bytes(paths: &JournalStorePaths, plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let recipients = recipient_keys(&read_recipients(paths)?)?;
    encrypt_to_recipients(&recipients, plaintext)
}

/// Encrypt a freshly created entry to every store recipient, plus this device's
/// own key when unlocked — so the authoring device can always re-read what it
/// wrote, even a joining device whose key isn't yet an approved recipient.
pub fn encrypt_new_entry(
    paths: &JournalStorePaths,
    plaintext: &[u8],
    identity: Option<&UnlockedIdentity>,
) -> AppResult<Vec<u8>> {
    let mut recipients = recipient_keys(&read_recipients(paths)?)?;
    if let Some(identity) = identity {
        let own = identity.recipient();
        if !recipients.iter().any(|r| r.to_string() == own.to_string()) {
            recipients.push(own);
        }
    }
    encrypt_to_recipients(&recipients, plaintext)
}

/// Append a recipient and return the new full list. Rejects a key or name that
/// already exists so a device can't be added twice or shadow another's label.
pub fn add_recipient(paths: &JournalStorePaths, recipient: Recipient) -> AppResult<Vec<Recipient>> {
    if x25519::Recipient::from_str(&recipient.key).is_err() {
        return Err(format!("'{}' is not a valid age recipient", recipient.key).into());
    }
    let mut recipients = read_recipients(paths)?;
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
    recipients.push(recipient);
    write_recipients(paths, &recipients)?;
    Ok(recipients)
}

/// Remove the recipient with `name` and return the new list. Refuses to remove
/// the last recipient, which would leave the store impossible to re-encrypt.
pub fn remove_recipient(paths: &JournalStorePaths, name: &str) -> AppResult<Vec<Recipient>> {
    let mut recipients = read_recipients(paths)?;
    let before = recipients.len();
    recipients.retain(|recipient| recipient.name != name);
    if recipients.len() == before {
        return Err(format!("no recipient named '{name}'").into());
    }
    if recipients.is_empty() {
        return Err("cannot remove the last recipient; the store would become unreadable".into());
    }
    write_recipients(paths, &recipients)?;
    Ok(recipients)
}

/// Relabel a recipient without changing its key. No re-encryption needed.
pub fn rename_recipient(paths: &JournalStorePaths, old: &str, new: &str) -> AppResult<()> {
    if new.trim().is_empty() {
        return Err("new recipient name cannot be empty".into());
    }
    let mut recipients = read_recipients(paths)?;
    if recipients.iter().any(|recipient| recipient.name == new) {
        return Err(format!("a recipient named '{new}' already exists").into());
    }
    let Some(target) = recipients
        .iter_mut()
        .find(|recipient| recipient.name == old)
    else {
        return Err(format!("no recipient named '{old}'").into());
    };
    target.name = new.to_string();
    write_recipients(paths, &recipients)?;
    Ok(())
}

/// Whether `identity`'s public key is one of the store's current recipients —
/// i.e. this device can already decrypt the store, and so is allowed to
/// re-encrypt it when approving or removing another device.
pub fn identity_is_recipient(
    paths: &JournalStorePaths,
    identity: &UnlockedIdentity,
) -> AppResult<bool> {
    let own = identity.recipient().to_string();
    Ok(read_recipients(paths)?
        .iter()
        .any(|recipient| recipient.key == own))
}

/// The pending join requests in the shared `.age/` folder, sorted by name.
pub fn read_pending(paths: &JournalStorePaths) -> AppResult<Vec<PendingRequest>> {
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
        requests.push(PendingRequest {
            id: id.to_string(),
            recipient: parsed.recipient,
        });
    }
    requests.sort_by(|a, b| a.recipient.name.cmp(&b.recipient.name));
    Ok(requests)
}

/// Delete a processed join request. A no-op if it's already gone.
pub fn remove_pending(paths: &JournalStorePaths, id: &str) -> AppResult<()> {
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
    paths: &JournalStorePaths,
    current: Option<&SecretString>,
    new: Option<&SecretString>,
) -> AppResult<()> {
    if matches!(new, Some(passphrase) if passphrase.expose_secret().is_empty()) {
        return Err("encryption passphrase cannot be empty".into());
    }
    let stored = read_stored_identity(&paths.identity_file)?;
    let identity = decrypt_identity(paths, current)?;
    write_stored_identity(paths, &stored.device_name, &identity, new)
}

/// Generate a fresh keypair for this device and add it to `recipients.toml`
/// *alongside* the current key (same name), returning the new public
/// [`Recipient`] and its not-yet-persisted unlocked identity. The old key stays a
/// recipient until [`drop_old_recipient`], so re-encryption during a rotation
/// can't lock this device out mid-way.
pub fn rotate_add_new_key(
    paths: &JournalStorePaths,
    old: &UnlockedIdentity,
) -> AppResult<(Recipient, UnlockedIdentity)> {
    let old_key = old.public_key();
    let mut recipients = read_recipients(paths)?;
    if !recipients.iter().any(|recipient| recipient.key == old_key) {
        return Err("this device is not a current recipient; cannot rotate".into());
    }

    let stored = read_stored_identity(&paths.identity_file)?;
    let new_identity = x25519::Identity::generate();
    let recipient = Recipient {
        name: stored.device_name,
        key: new_identity.to_public().to_string(),
    };
    recipients.push(recipient.clone());
    write_recipients(paths, &recipients)?;
    Ok((
        recipient,
        UnlockedIdentity {
            identity: new_identity,
        },
    ))
}

/// Persist the rotated identity as this device's key file, preserving its
/// passphrase state (`passphrase` re-wraps it, `None` stores it plaintext).
pub fn commit_rotated_identity(
    paths: &JournalStorePaths,
    recipient: &Recipient,
    identity: &UnlockedIdentity,
    passphrase: Option<&SecretString>,
) -> AppResult<()> {
    write_stored_identity(paths, &recipient.name, &identity.identity, passphrase)
}

/// Remove the retired key from `recipients.toml` (the final step of a rotation,
/// after every entry has been re-encrypted to the new key).
pub fn drop_old_recipient(paths: &JournalStorePaths, old_key: &str) -> AppResult<()> {
    let mut recipients = read_recipients(paths)?;
    recipients.retain(|recipient| recipient.key != old_key);
    write_recipients(paths, &recipients)?;
    Ok(())
}

fn create_device_identity(
    paths: &JournalStorePaths,
    name: &str,
    passphrase: Option<&SecretString>,
) -> AppResult<Recipient> {
    if name.trim().is_empty() {
        return Err("device name cannot be empty".into());
    }

    let identity = x25519::Identity::generate();
    let recipient = Recipient {
        name: name.to_string(),
        key: identity.to_public().to_string(),
    };
    write_stored_identity(paths, name, &identity, passphrase)?;
    Ok(recipient)
}

/// Write this device's identity file, scrypt-wrapping the private key when a
/// passphrase is given and storing it plaintext (mode 0600) otherwise.
fn write_stored_identity(
    paths: &JournalStorePaths,
    name: &str,
    identity: &x25519::Identity,
    passphrase: Option<&SecretString>,
) -> AppResult<()> {
    if matches!(passphrase, Some(passphrase) if passphrase.expose_secret().is_empty()) {
        return Err("encryption passphrase cannot be empty".into());
    }
    let stored = StoredIdentity {
        device_name: name.to_string(),
        encrypted_identity: match passphrase {
            Some(passphrase) => Some(encrypt_identity(identity, passphrase)?),
            None => None,
        },
        plain_identity: match passphrase {
            Some(_) => None,
            None => Some(Zeroizing::new(
                identity.to_string().expose_secret().to_string(),
            )),
        },
    };
    // The serialized document carries the plaintext key in the no-passphrase
    // case; zeroize the buffer once it's on disk.
    let serialized = Zeroizing::new(toml::to_string_pretty(&stored)?);
    write_private_file(&paths.identity_file, serialized.as_bytes())
}

/// Read this device's identity file verbatim, for snapshotting before a rotation
/// so it can be put back byte-for-byte if the rotation fails.
pub fn read_identity_file_bytes(paths: &JournalStorePaths) -> AppResult<Vec<u8>> {
    Ok(fs::read(&paths.identity_file)?)
}

/// Restore this device's identity file from bytes captured by
/// [`read_identity_file_bytes`], preserving the private-file mode (0600).
pub fn restore_identity_file(paths: &JournalStorePaths, bytes: &[u8]) -> AppResult<()> {
    write_private_file(&paths.identity_file, bytes)
}

fn recipient_keys(recipients: &[Recipient]) -> AppResult<Vec<x25519::Recipient>> {
    if recipients.is_empty() {
        return Err("journal encryption recipients file is empty".into());
    }
    recipients
        .iter()
        .map(|recipient| Ok(x25519::Recipient::from_str(&recipient.key)?))
        .collect()
}

fn encrypt_to_recipients(recipients: &[x25519::Recipient], plaintext: &[u8]) -> AppResult<Vec<u8>> {
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
) -> AppResult<Vec<u8>> {
    Ok(age::decrypt(identity, ciphertext)?)
}

fn encrypt_identity(identity: &x25519::Identity, passphrase: &SecretString) -> AppResult<Vec<u8>> {
    let recipient = age::scrypt::Recipient::new(passphrase.clone());
    Ok(age::encrypt(
        &recipient,
        identity.to_string().expose_secret().as_bytes(),
    )?)
}

fn decrypt_identity(
    paths: &JournalStorePaths,
    passphrase: Option<&SecretString>,
) -> AppResult<x25519::Identity> {
    let stored = read_stored_identity(&paths.identity_file)?;
    // The decrypted secret key lives in this string; zeroize it on drop so it
    // doesn't linger in freed heap after we parse it into an identity.
    let text: Zeroizing<String> = match (&stored.encrypted_identity, &stored.plain_identity) {
        (Some(blob), _) => {
            let passphrase = passphrase
                .ok_or("journal identity is passphrase-protected; a passphrase is required")?;
            let identity = age::scrypt::Identity::new(passphrase.clone());
            Zeroizing::new(String::from_utf8(age::decrypt(&identity, blob)?)?)
        }
        (None, Some(plain)) => plain.clone(),
        (None, None) => return Err("journal identity file has no key material".into()),
    };
    Ok(x25519::Identity::from_str(text.trim())?)
}

fn read_stored_identity(path: &Path) -> AppResult<StoredIdentity> {
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

/// Comment header written above the recipients table. TOML ignores `#` lines, so
/// this stays parse-safe while telling anyone who opens the file that it is
/// app-managed and holds non-secret public keys.
const RECIPIENTS_HEADER: &str = "\
# Managed by journal — do not edit or delete.
# These are your age encryption recipients (public keys, not secrets).
# Each device that can read this journal has one entry below.
";

fn write_recipients(paths: &JournalStorePaths, recipients: &[Recipient]) -> AppResult<()> {
    fs::create_dir_all(&paths.age_dir)?;
    let document = RecipientsFileRef {
        recipient: recipients,
    };
    let body = format!(
        "{RECIPIENTS_HEADER}\n{}",
        toml::to_string_pretty(&document)?
    );
    atomic_write(&paths.recipients_file, body.as_bytes())
}

fn write_pending(paths: &JournalStorePaths, recipient: &Recipient) -> AppResult<()> {
    fs::create_dir_all(&paths.age_dir)?;
    let document = PendingFileRef { recipient };
    let path = paths.age_dir.join(pending_file_name(&recipient.key));
    atomic_write(&path, toml::to_string_pretty(&document)?.as_bytes())
}

/// Write `content` to `path` via a sibling temp file plus rename, so a crash
/// mid-write can't truncate an existing `recipients.toml` (which would strand
/// every device) or leave a half-written join request behind.
fn atomic_write(path: &Path, content: &[u8]) -> AppResult<()> {
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

    fn paths_in(dir: &Path) -> JournalStorePaths {
        JournalStorePaths::for_config(&dir.join("config.toml"), &dir.join("journals")).unwrap()
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
        let phone = JournalStorePaths::for_config(
            &dir.path().join("phone").join("config.toml"),
            &dir.path().join("journals"),
        )
        .unwrap();

        initialize_store_identity(&laptop, "laptop", Some(&SecretString::from("pw")))
            .unwrap();
        let phone_recipient = request_store_access(&phone, "phone", None).unwrap();
        add_recipient(&laptop, phone_recipient).unwrap();

        let ciphertext = encrypt_bytes(&laptop, b"shared secret").unwrap();
        let laptop_id = unlock_identity(&laptop, Some(&SecretString::from("pw"))).unwrap();
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
        let phone = JournalStorePaths::for_config(
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

        // Same key → rejected for the key clash.
        assert!(add_recipient(&paths, recipient.clone()).is_err());
        // Same name, different (valid) key → rejected for the name clash.
        let same_name_new_key = Recipient {
            key: "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsuaxjx".to_string(),
            ..recipient
        };
        assert!(add_recipient(&paths, same_name_new_key).is_err());
    }

    #[test]
    fn remove_recipient_refuses_the_last_one() {
        let dir = tempdir().unwrap();
        let paths = paths_in(dir.path());
        initialize_store_identity(&paths, "laptop", Some(&SecretString::from("pw")))
            .unwrap();

        assert!(remove_recipient(&paths, "laptop").is_err());
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
    ) -> AppResult<Vec<u8>> {
        decrypt_bytes_with_identity(ciphertext, &identity.identity)
    }
}
