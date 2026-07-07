use std::path::PathBuf;

use thiserror::Error;

/// Semantically-meaningful storage failures that in-process callers may want to
/// branch on (e.g. to prompt for a passphrase rather than show a generic
/// error). Incidental IO and crypto failures stay as boxed [`AppResult`] errors.
///
/// [`AppResult`]: crate::AppResult
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorageError {
    /// An encrypted item was accessed without an unlocked identity. `context`
    /// names what was being read (e.g. `"entry"`, `"asset"`, `"store"`).
    #[error("encrypted {context} requires an unlocked journal encryption identity")]
    LockedIdentity { context: &'static str },

    /// Encrypted entries exist but the signed device roster needed to encrypt
    /// more is gone — continuing could leave the store partially encrypted.
    #[error(
        "encrypted entries already exist but the device roster is missing at {}; cannot safely continue encryption",
        path.display()
    )]
    RecipientsMissing { path: PathBuf },

    /// A move would overwrite an existing path. `what` names the destination
    /// (e.g. `"asset trash destination"`).
    #[error("{what} already exists: {}", path.display())]
    TargetExists { what: &'static str, path: PathBuf },

    /// The signed device roster failed verification: a forged/unauthorized op, a
    /// broken signature chain, a changed genesis, or a rolled-back history. The
    /// store refuses to encrypt or decrypt to an untrusted recipient set, so this
    /// is surfaced rather than silently trusting the tampered file. `detail`
    /// explains which check failed.
    #[error("device roster failed verification: {detail}")]
    RosterUnverified { detail: String },
}
