#![forbid(unsafe_code)]

mod cli;
mod config;
mod device;
mod encryption_cli;
mod licenses;
mod prompts;
mod tui;

pub(crate) type AppResult<T> = anyhow::Result<T>;

pub fn run() -> anyhow::Result<()> {
    cli::run()
}

/// The command a device runs to request access to an already-encrypted store.
/// Referenced from CLI errors and the TUI enroll notice so the wording lives in
/// one place.
pub(crate) const ENROLL_CMD: &str = "notema encryption device enroll";

/// The command an approving device runs to admit a pending join request. A
/// device name is appended when one is known. Shared by the CLI prompts and the
/// TUI awaiting-approval notice so the wording lives in one place.
pub(crate) const APPROVE_CMD: &str = "notema encryption device approve";
