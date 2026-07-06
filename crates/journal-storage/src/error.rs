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

    /// Encrypted entries exist but the recipients file needed to encrypt more is
    /// gone — continuing could leave the store partially encrypted.
    #[error(
        "encrypted entries already exist but recipients file is missing at {}; cannot safely continue encryption",
        path.display()
    )]
    RecipientsMissing { path: PathBuf },

    /// A move would overwrite an existing path. `what` names the destination
    /// (e.g. `"asset trash destination"`).
    #[error("{what} already exists: {}", path.display())]
    TargetExists { what: &'static str, path: PathBuf },
}
