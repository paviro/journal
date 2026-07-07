use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, EncryptionError>;

/// A failure in the encryption layer. The first three variants carry state a
/// caller acts on (prompt for a passphrase, refuse to continue); the domain
/// variants are validation failures surfaced to the user; the rest wrap an
/// underlying failure, typed rather than boxed.
#[derive(Debug, Error)]
pub enum EncryptionError {
    /// An encrypted item was accessed without an unlocked identity. `context` is
    /// a caller-supplied label for what needed the identity (e.g. `"entry"`,
    /// `"asset"`, `"approve"`).
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

    /// A store already has a device roster, so it can't be initialized again
    /// (a second genesis would brick decryption for the existing devices).
    #[error("device roster already exists; use request_store_access to join instead")]
    RosterExists,

    /// An operation needed at least one recipient but the roster is empty.
    #[error("journal encryption recipients file is empty")]
    NoRecipients,

    /// A recipient with this age key is already on the roster.
    #[error("recipient '{name}' is already present")]
    RecipientExists { name: String },

    /// A recipient with this name is already on the roster.
    #[error("a recipient named '{name}' already exists; pick a unique name")]
    RecipientNameTaken { name: String },

    /// No recipient on the roster carries this name.
    #[error("no recipient named '{name}'")]
    UnknownRecipient { name: String },

    /// Revoking this recipient would leave the store with none, making it
    /// impossible to re-encrypt.
    #[error("cannot revoke the last recipient; the store would become unreadable")]
    LastRecipient,

    /// This device's key isn't a current recipient, so it can't rotate.
    #[error("this device is not a current recipient; cannot rotate")]
    NotARecipient,

    /// A recipient carries a malformed age (X25519) public key.
    #[error("'{key}' is not a valid age recipient")]
    InvalidRecipientKey { key: String },

    /// A recipient carries a malformed Ed25519 signing key.
    #[error("'{key}' is not a valid signing key")]
    InvalidSigningKey { key: String },

    /// A device name was blank.
    #[error("device name cannot be empty")]
    EmptyDeviceName,

    /// A recipient rename target was blank.
    #[error("recipient name cannot be empty")]
    EmptyRecipientName,

    /// A passphrase was blank.
    #[error("encryption passphrase cannot be empty")]
    EmptyPassphrase,

    /// The stored identity is passphrase-protected but no passphrase was given.
    #[error("journal identity is passphrase-protected; a passphrase is required")]
    PassphraseRequired,

    /// The unlocked identity failed its self round-trip check.
    #[error("journal encryption identity check failed")]
    IdentityCheckFailed,

    /// The stored identity's key material could not be parsed (wrong length or
    /// not a valid age key).
    #[error("journal identity key material is malformed")]
    MalformedStoredIdentity,

    /// The config path has no parent directory to derive key locations from.
    #[error("config path has no parent directory")]
    MissingConfigParent,

    /// The OS randomness source failed while generating a signing key.
    #[error("failed to gather randomness for signing key: {0}")]
    Randomness(String),

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
