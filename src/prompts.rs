//! Interactive terminal prompts shared by the CLI encryption commands and
//! first-run setup, kept together so their wording stays consistent.

use crate::AppResult;
use journal_storage::{ExposeSecret, SecretString};
use rpassword::prompt_password;
use std::io::{self, IsTerminal, Write};

/// Ask the user to confirm a destructive encryption operation, returning `true`
/// to proceed. `skip` (from `--yes`) bypasses the prompt. Without a terminal to
/// answer on, it refuses rather than blocking, pointing at `--yes`.
pub(crate) fn confirm(prompt: &str, skip: bool) -> AppResult<bool> {
    if skip {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Err(format!(
            "{prompt}\nrefusing to continue without a terminal to confirm; re-run with --yes to proceed"
        )
        .into());
    }
    print!("{prompt} [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
}

/// Resolve the device name and optional passphrase for a *new* identity,
/// reusing the first-run prompts. `name` skips the name prompt; `no_passphrase`
/// stores the key unprotected, otherwise the passphrase is chosen interactively.
pub(crate) fn resolve_new_identity_options(
    name: Option<&str>,
    no_passphrase: bool,
) -> AppResult<(String, Option<SecretString>)> {
    let mut stdout = io::stdout();
    let device_name = match name {
        Some(name) => name.to_string(),
        None => prompt_device_name(&mut stdout)?,
    };
    let passphrase = if no_passphrase {
        None
    } else {
        prompt_passphrase_choice(&mut stdout)?
    };
    Ok((device_name, passphrase))
}

/// Prompt for this device's name (used to label its key), defaulting to the
/// hostname.
pub(crate) fn prompt_device_name(stdout: &mut impl Write) -> AppResult<String> {
    let default_name = crate::device::default_device_name();
    write!(stdout, "Device name [{default_name}]: ")?;
    stdout.flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let name = input.trim();
    Ok(if name.is_empty() {
        default_name
    } else {
        name.to_string()
    })
}

/// Ask whether to protect the key with a passphrase, returning the passphrase to
/// use (`None` = store the key unprotected). Defaults to yes.
pub(crate) fn prompt_passphrase_choice(stdout: &mut impl Write) -> AppResult<Option<SecretString>> {
    writeln!(stdout, "Protect the key with a passphrase?")?;
    writeln!(
        stdout,
        "  Yes — key is encrypted at rest; you enter the passphrase to unlock (best for laptops)."
    )?;
    writeln!(
        stdout,
        "  No  — key opens automatically; relies on this device's own security (phones with full-disk encryption, etc.)."
    )?;
    write!(stdout, "Use a passphrase? [Y/n]: ")?;
    stdout.flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if matches!(input.trim(), "n" | "N" | "no" | "NO" | "No") {
        Ok(None)
    } else {
        Ok(Some(prompt_new_passphrase()?))
    }
}

/// Prompt for a new passphrase twice, rejecting an empty entry or a mismatch.
pub(crate) fn prompt_new_passphrase() -> AppResult<SecretString> {
    let passphrase = SecretString::from(prompt_password("New journal encryption passphrase: ")?);
    if passphrase.expose_secret().is_empty() {
        return Err("encryption passphrase cannot be empty".into());
    }
    let confirm = SecretString::from(prompt_password("Confirm journal encryption passphrase: ")?);
    if passphrase.expose_secret() != confirm.expose_secret() {
        return Err("encryption passphrases did not match".into());
    }
    Ok(passphrase)
}

/// Prompt once for an existing passphrase to unlock this device's identity.
pub(crate) fn prompt_unlock_passphrase() -> AppResult<SecretString> {
    Ok(SecretString::from(prompt_password(
        "Journal encryption passphrase: ",
    )?))
}
