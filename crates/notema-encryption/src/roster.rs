//! The store's authenticated membership: a signed, append-only log of device
//! operations (`.age/devices.toml`) rooted at a self-signed genesis op.
//!
//! Every roster change is an Ed25519-signed op authorized by a device that is
//! *already* trusted at that point in the log, and each op commits to the hash of
//! the one before it. Verification replays from the genesis and **fails closed**:
//! a forged op, a broken chain, a swapped genesis, or a rolled-back history all
//! reject the whole file rather than let an unauthenticated recipient slip in.
//! This is what an attacker with write access to the synced folder cannot beat —
//! they hold no trusted signing key, so any recipient they inject fails to verify.
//!
//! Trust is anchored by two per-device pins kept locally in the (never-synced)
//! `devices-trust.toml`: the genesis fingerprint (detects a replaced root) and
//! the last-seen head hash (detects a rolled-back / truncated log).
//!
//! # Residual threats (accepted by design)
//!
//! Three gaps can't be closed by signatures alone over an untrusted shared folder,
//! and are conscious trade-offs for a serverless, single-owner journal:
//!
//! 1. **Pending-request injection.** Anyone who can write the folder can drop a
//!    `pending-<id>.toml` join request; a request legitimately originates from an
//!    untrusted, not-yet-trusted device, so it can't be authenticated. It grants
//!    nothing until a human approves it, and approval is gated on an out-of-band
//!    fingerprint check (shown at approval time) — that is the real defence, not
//!    crypto. The self-signature on the request only proves key possession and
//!    weeds out corruption, not malice.
//! 2. **Equivocation / rollback.** The sync host can serve a truncated or forked
//!    log (e.g. hiding a revocation). Head-pinning detects this for a device that
//!    already saw the newer state, but a brand-new device on its first sync has no
//!    pin to compare against (trust on first use) — which is why the genesis
//!    fingerprint is confirmed out of band when a device joins.
//! 3. **Entry/attachment forgery.** The roster authenticates *membership*, not
//!    *content*: individual entries and assets are encrypted but not signed, and
//!    carry no authorship. Recipient public keys are public in the roster, so
//!    anyone who can write the folder can encrypt to them — forging brand-new
//!    entries/attachments or replacing existing ones wholesale, undetected. (age's
//!    per-file AEAD only catches bit-level corruption of a given ciphertext, not
//!    substitution of a freshly forged one.) This is accepted because the threat
//!    model is confidentiality against untrusted storage, not defending content
//!    integrity against a writer who could equally just delete entries.

use crate::{EncryptionError, Recipient, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

/// Domain separator mixed into every signed op so a roster signature can never be
/// confused with any other Ed25519 signature the app might make over other data.
const DOMAIN: &[u8] = b"notema.roster.v1";

/// The kind of a roster operation. Serializes to the lowercase variant name, and
/// its [`OpKind::as_bytes`] feeds the signed op bytes — so the wire strings must
/// stay `genesis`/`add`/`revoke`/`rename` for existing rosters to keep verifying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpKind {
    Genesis,
    Add,
    Revoke,
    Rename,
}

impl OpKind {
    fn as_bytes(self) -> &'static [u8] {
        match self {
            OpKind::Genesis => b"genesis",
            OpKind::Add => b"add",
            OpKind::Revoke => b"revoke",
            OpKind::Rename => b"rename",
        }
    }
}

/// Comment header written above the op log. TOML ignores `#` lines, so this stays
/// parse-safe while telling anyone who opens the file that it is app-managed and
/// signed — editing it by hand breaks the signature chain and locks the store.
const DEVICES_HEADER: &str = "\
# Managed by Notema — do not edit or delete.
# The devices allowed to read this journal, as a signed append-only log.
# Every operation is signed by a device already trusted at that point; editing this
# file by hand breaks the chain and the store will refuse to open.
";

/// One signed operation in the log. `enc_key`/`sign_key` name the *subject* device the
/// op acts on; `signer_key`/`sig` are the *authorization* (an already-trusted device's
/// public key and its Ed25519 signature over the op's canonical bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterOp {
    pub seq: u64,
    /// Hex SHA-256 of the previous op's canonical bytes; empty for the genesis.
    pub prev_hash: String,
    pub kind: OpKind,
    pub name: String,
    /// The subject device's age (X25519) recipient — an `age1…` public key.
    pub enc_key: String,
    /// The subject device's signing key, `ed25519:<hex>`.
    pub sign_key: String,
    /// The authorizing device's signing key, `ed25519:<hex>`. Equals `sign_key` for a
    /// self-signed genesis.
    pub signer_key: String,
    /// Hex Ed25519 signature by `signer_key` over [`RosterOp::signing_bytes`].
    pub sig: String,
}

impl RosterOp {
    /// The exact bytes covered by `sig`: a domain-separated, length-prefixed
    /// concatenation of every field except the signature itself. Explicit framing
    /// (not the TOML text) so formatting or field reordering can never change what
    /// was signed.
    fn signing_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        push_field(&mut buf, DOMAIN);
        buf.extend_from_slice(&self.seq.to_le_bytes());
        push_field(&mut buf, self.prev_hash.as_bytes());
        push_field(&mut buf, self.kind.as_bytes());
        push_field(&mut buf, self.name.as_bytes());
        push_field(&mut buf, self.enc_key.as_bytes());
        push_field(&mut buf, self.sign_key.as_bytes());
        push_field(&mut buf, self.signer_key.as_bytes());
        buf
    }

    /// This op's position in the chain: hex SHA-256 of its canonical bytes. The
    /// next op's `prev_hash` and the head pin are both this value.
    fn hash(&self) -> String {
        hex::encode(Sha256::digest(self.signing_bytes()))
    }

    fn recipient(&self) -> Recipient {
        Recipient {
            name: self.name.clone(),
            enc_key: self.enc_key.clone(),
            sign_key: self.sign_key.clone(),
        }
    }
}

/// Append one length-prefixed field to a signing buffer: a `u32` little-endian
/// length followed by the bytes. Shared by every signed payload in the crate so
/// there is a single, unambiguous framing (fields can't run together or be
/// reordered without changing the bytes).
pub(crate) fn push_field(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// The local, per-device trust anchors. Both `None` on a device that has not yet
/// seen a valid roster (first sync / store creation) — the trust-on-first-use case.
#[derive(Debug, Clone, Default)]
pub struct TrustPins {
    pub genesis_hash: Option<String>,
    pub head_hash: Option<String>,
}

/// The outcome of a successful [`verify`]: the current recipient set plus the
/// genesis/head hashes to pin.
#[derive(Debug, Clone)]
pub struct Verified {
    pub recipients: Vec<Recipient>,
    pub genesis_hash: String,
    pub head_hash: String,
}

/// Replay and authenticate the whole log against the local pins. Returns the
/// current recipient set on success, or [`EncryptionError::RosterUnverified`] on any
/// failure — callers must treat that as "do not encrypt/decrypt".
pub fn verify(ops: &[RosterOp], pins: &TrustPins) -> Result<Verified> {
    let Some(genesis_op) = ops.first() else {
        return Err(unverified("the roster is empty"));
    };
    if genesis_op.kind != OpKind::Genesis || genesis_op.seq != 0 || !genesis_op.prev_hash.is_empty()
    {
        return Err(unverified("the roster does not start with a genesis op"));
    }
    if genesis_op.signer_key != genesis_op.sign_key {
        return Err(unverified("the genesis op is not self-signed"));
    }
    verify_op_sig(genesis_op)?;

    let genesis_hash = genesis_op.hash();
    if let Some(pinned) = &pins.genesis_hash
        && pinned != &genesis_hash
    {
        return Err(unverified(
            "the genesis has changed since this device last synced (a replaced root)",
        ));
    }

    let mut trusted: Vec<String> = vec![genesis_op.sign_key.clone()];
    let mut recipients: Vec<Recipient> = vec![genesis_op.recipient()];
    let mut hashes: Vec<String> = vec![genesis_hash.clone()];

    for (index, op) in ops.iter().enumerate().skip(1) {
        if op.seq != index as u64 {
            return Err(unverified("an op is out of sequence"));
        }
        if op.prev_hash != hashes[index - 1] {
            return Err(unverified("the signature chain is broken"));
        }
        if !trusted.iter().any(|key| key == &op.signer_key) {
            return Err(unverified(
                "an op is signed by a device that was not trusted",
            ));
        }
        verify_op_sig(op)?;

        match op.kind {
            OpKind::Add => {
                if !recipients.iter().any(|r| r.enc_key == op.enc_key) {
                    recipients.push(op.recipient());
                }
                if !trusted.iter().any(|key| key == &op.sign_key) {
                    trusted.push(op.sign_key.clone());
                }
            }
            OpKind::Revoke => {
                recipients.retain(|r| r.enc_key != op.enc_key);
                trusted.retain(|key| key != &op.sign_key);
            }
            OpKind::Rename => {
                if let Some(target) = recipients.iter_mut().find(|r| r.enc_key == op.enc_key) {
                    target.name = op.name.clone();
                }
            }
            OpKind::Genesis => return Err(unverified("a second genesis op appears in the log")),
        }
        hashes.push(op.hash());
    }

    if recipients.is_empty() {
        return Err(unverified("the roster has no recipients left"));
    }

    // Rollback / truncation detection: a previously-seen head must still be part
    // of this (append-only) chain. If it's gone, the log was rewound.
    if let Some(pinned) = &pins.head_hash
        && !hashes.iter().any(|hash| hash == pinned)
    {
        return Err(unverified(
            "the roster no longer includes a state this device already saw (a rollback)",
        ));
    }

    Ok(Verified {
        recipients,
        genesis_hash,
        head_hash: hashes
            .into_iter()
            .next_back()
            .expect("genesis pushed above"),
    })
}

/// Append a new op to the log, signed by `signer_key` via `sign_bytes`, and write
/// the file back. `sign_bytes` produces the hex Ed25519 signature over the op's
/// canonical bytes (supplied by the caller's unlocked identity).
pub fn append(
    path: &Path,
    kind: OpKind,
    name: &str,
    enc_key: &str,
    sign_key: &str,
    signer_key: &str,
    sign_bytes: impl FnOnce(&[u8]) -> String,
) -> Result<RosterOp> {
    let ops = read_ops(path)?;
    let (seq, prev_hash) = match ops.last() {
        Some(last) => (last.seq + 1, last.hash()),
        None => (0, String::new()),
    };
    let mut op = RosterOp {
        seq,
        prev_hash,
        kind,
        name: name.to_string(),
        enc_key: enc_key.to_string(),
        sign_key: sign_key.to_string(),
        signer_key: signer_key.to_string(),
        sig: String::new(),
    };
    op.sig = sign_bytes(&op.signing_bytes());

    let mut all = ops;
    all.push(op.clone());
    write_ops(path, &all)?;
    Ok(op)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RosterFile {
    #[serde(default, rename = "operation")]
    ops: Vec<RosterOp>,
}

#[derive(Serialize)]
struct RosterFileRef<'a> {
    #[serde(rename = "operation")]
    operations: &'a [RosterOp],
}

/// The raw ops in file order, or empty when the store isn't encrypted.
pub fn read_ops(path: &Path) -> Result<Vec<RosterOp>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str::<RosterFile>(&text)?.ops)
}

fn write_ops(path: &Path, ops: &[RosterOp]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let document = RosterFileRef { operations: ops };
    let body = format!("{DEVICES_HEADER}\n{}", toml::to_string_pretty(&document)?);
    crate::atomic_write(path, body.as_bytes())
}

// --- local trust pins -------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinsFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    genesis_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    head_hash: Option<String>,
}

pub fn read_pins(path: &Path) -> Result<TrustPins> {
    if !path.exists() {
        return Ok(TrustPins::default());
    }
    let parsed: PinsFile = toml::from_str(&fs::read_to_string(path)?)?;
    Ok(TrustPins {
        genesis_hash: parsed.genesis_hash,
        head_hash: parsed.head_hash,
    })
}

pub fn write_pins(path: &Path, genesis_hash: &str, head_hash: &str) -> Result<()> {
    let document = PinsFile {
        genesis_hash: Some(genesis_hash.to_string()),
        head_hash: Some(head_hash.to_string()),
    };
    crate::atomic_write(path, toml::to_string_pretty(&document)?.as_bytes())
}

/// A short, human-comparable fingerprint of a device, covering *both* its
/// encryption and signing keys so tampering with either shows up. Displayed at
/// approval time for an out-of-band check against what the joining device shows.
pub fn fingerprint(enc_key: &str, sign_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(enc_key.as_bytes());
    hasher.update([0u8]);
    hasher.update(sign_key.as_bytes());
    let digest = hex::encode(hasher.finalize());
    digest
        .as_bytes()
        .chunks(4)
        .take(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

fn verify_op_sig(op: &RosterOp) -> Result<()> {
    if crate::signing::verify_signature(&op.signer_key, &op.signing_bytes(), &op.sig) {
        Ok(())
    } else {
        Err(unverified(&format!(
            "op #{} has an invalid signature",
            op.seq
        )))
    }
}

fn unverified(detail: &str) -> EncryptionError {
    EncryptionError::RosterUnverified {
        detail: detail.to_string(),
    }
}
