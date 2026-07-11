//! Best-effort device naming for labelling encryption keys. The name is not a
//! secret or a security boundary — it only helps a human tell their devices
//! apart in `notema encryption device list`, so lookup falls back gracefully
//! rather than failing.

use std::process::Command;

/// A human-friendly default device name (the hostname), or `"this device"`.
pub fn default_device_name() -> String {
    hostname()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this device".to_string())
}

fn hostname() -> Option<String> {
    command_output("hostname", &[]).filter(|name| !name.trim().is_empty())
}

/// Run a command and return its trimmed stdout, or `None` if it can't run or
/// exits non-zero. Used only for best-effort identity probing.
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}
