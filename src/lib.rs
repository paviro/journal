pub mod cli;
pub mod config;
pub mod device;
pub mod editor;
pub mod encryption_cli;
pub mod prompts;
pub mod tui;

pub use journal_core::AppResult;

/// The command a device runs to request access to an already-encrypted store.
/// Referenced from CLI errors and the TUI enroll notice so the wording lives in
/// one place.
pub(crate) const ENROLL_CMD: &str = "journal encryption device enroll";
