use crate::files::atomic_write;
use crate::identity::{UnlockedIdentity, create_device_identity};
use crate::signing::{sign_bytes, verify_signature};
use crate::{KeyPaths, Recipient, Result, roster};
use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::fs;

/// A join request waiting to be approved: the requesting device's [`Recipient`]
/// and the stable `id` derived from its key (the `pending-<id>.toml` file name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub id: String,
    pub recipient: Recipient,
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
            &parsed.recipient.sign_key,
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
    let path = paths.age_dir.join(pending_file_name(&recipient.enc_key));
    atomic_write(&path, toml::to_string_pretty(&document)?.as_bytes())
}

/// Build the byte buffer a device self-signs in its join request, binding its
/// name and both public keys under a distinct domain so it can't be replayed as
/// any other signature.
fn pending_signing_bytes(recipient: &Recipient) -> Vec<u8> {
    let mut buf = Vec::new();
    roster::push_field(&mut buf, b"journal.pending.v1");
    roster::push_field(&mut buf, recipient.name.as_bytes());
    roster::push_field(&mut buf, recipient.enc_key.as_bytes());
    roster::push_field(&mut buf, recipient.sign_key.as_bytes());
    buf
}

/// The `pending-<id>.toml` file name for a recipient, where `<id>` is a stable,
/// filename-safe slice of the bech32 public key (unique enough to avoid
/// collisions between devices, deterministic so a re-run overwrites its own).
fn pending_file_name(enc_key: &str) -> String {
    let id: String = enc_key
        .strip_prefix("age1")
        .unwrap_or(enc_key)
        .chars()
        .take(12)
        .collect();
    format!("pending-{id}.toml")
}
