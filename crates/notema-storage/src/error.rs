use std::path::PathBuf;

use thiserror::Error;

/// Semantically-meaningful storage failures that in-process callers may want to
/// branch on. Incidental I/O failures stay as boxed errors, and
/// encryption-specific failures live in `notema-encryption`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorageError {
    /// A move would overwrite an existing path. `what` names the destination
    /// (e.g. `"asset trash destination"`).
    #[error("{what} already exists: {}", path.display())]
    TargetExists { what: &'static str, path: PathBuf },
}
