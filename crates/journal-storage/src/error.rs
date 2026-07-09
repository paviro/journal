use std::path::PathBuf;

use thiserror::Error;

/// Semantically-meaningful storage failures that in-process callers may want to
/// branch on. Incidental IO failures stay as boxed [`AppResult`] errors, and
/// encryption-specific failures live in [`EncryptionError`].
///
/// [`AppResult`]: journal_core::AppResult
/// [`EncryptionError`]: crate::EncryptionError
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorageError {
    /// A move would overwrite an existing path. `what` names the destination
    /// (e.g. `"asset trash destination"`).
    #[error("{what} already exists: {}", path.display())]
    TargetExists { what: &'static str, path: PathBuf },
}
