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

mod cipher;
mod error;
mod files;
mod identity;
mod paths;
mod pending;
mod recipients;
mod roster;
mod signing;

#[cfg(test)]
mod tests;

pub use age::secrecy::{ExposeSecret, SecretString};

pub use cipher::{
    CiphertextBytes, EncryptionRecipients, PlaintextBytes, decrypt_file_bytes, encrypt_bytes,
    encrypt_new_entry, encrypt_to_file,
};
pub use error::{EncryptionError, Result};
pub use files::{atomic_write, sibling_temp_path};
pub use identity::{
    DeviceIdentityInfo, UnlockedIdentity, device_identity_info, read_identity_file_bytes,
    restore_identity_file, set_identity_passphrase, unlock_identity,
};
pub use paths::KeyPaths;
pub use pending::{PendingRequest, read_pending, remove_pending, request_store_access};
pub use recipients::{
    Recipient, add_recipient, advance_trust_pins, commit_rotated_identity, drop_old_recipient,
    identity_is_recipient, initialize_store_identity, read_recipients, rename_recipient,
    revoke_recipient, rotate_add_new_key,
};
