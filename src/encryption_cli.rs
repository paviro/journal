use crate::{AppResult, config::Config, prompts};
use anyhow::bail;
use indicatif::{ProgressBar, ProgressStyle};
use notema_storage::JournalStore;
use std::path::Path;

/// A progress sink for CLI migrations that drives an `indicatif` bar. A fresh
/// bar is created at the start of each pass (a `(0, total)` tick) — so a
/// two-pass operation like rotation shows a bar per pass — and cleared when the
/// pass completes.
pub(crate) fn cli_progress() -> impl FnMut(usize, usize) {
    let mut bar: Option<ProgressBar> = None;
    move |done, total| {
        if done == 0 {
            let fresh = ProgressBar::new(total as u64);
            fresh.set_style(
                ProgressStyle::with_template("{bar:40} {pos}/{len} files")
                    .unwrap_or_else(|_| ProgressStyle::default_bar()),
            );
            bar = Some(fresh);
        }
        if let Some(bar) = &bar {
            bar.set_position(done as u64);
            if total == 0 || done >= total {
                bar.finish_and_clear();
            }
        }
    }
}

pub(crate) fn encrypt_store(
    config_path: &Path,
    config: &Config,
    device_name: Option<&str>,
    no_passphrase: bool,
) -> AppResult<()> {
    let store = JournalStore::for_config(config_path, &config.journal.path)?;
    let recipient = if store.encryption_enabled() {
        if !store.unlock_available() {
            bail!(
                "this journal is already encrypted for other devices, but this one has no key at {}; run `{}` to request access instead",
                store.identity_path().display(),
                crate::ENROLL_CMD,
            );
        }
        store.public_recipient()?
    } else if store.has_encrypted_entries()? {
        // Encrypted entries but no roster to encrypt more against — reuse the
        // storage layer's own message rather than restating it here. anyhow
        // prints the typed error's Display, so route it through directly.
        return Err(notema_encryption::EncryptionError::RecipientsMissing {
            path: store.device_roster_path().to_path_buf(),
        }
        .into());
    } else {
        println!("No journal encryption identity configured; generating an age identity.");
        let (name, passphrase) = prompts::resolve_new_identity_options(device_name, no_passphrase)?;
        let summary = store.enable_encryption(&name, passphrase.as_ref(), cli_progress())?;
        let recipient = summary.recipient;
        println!(
            "Encrypted journal store at {}",
            config.journal.path.display()
        );
        println!(
            "Encryption recipient: {recipient}. Identity file: {}. Back it up; without it encrypted journal files cannot be decrypted.",
            store.identity_path().display()
        );
        if passphrase.is_none() {
            println!("This key has no passphrase — keep this device and its backups secure.");
        }
        return Ok(());
    };

    store.encrypt_store(cli_progress())?;
    println!(
        "Encrypted journal store at {}",
        config.journal.path.display()
    );
    println!(
        "Encryption recipient: {recipient}. Identity file: {}. Back it up; without it encrypted journal files cannot be decrypted.",
        store.identity_path().display()
    );
    Ok(())
}

pub(crate) fn decrypt_store(config_path: &Path, config: &Config) -> AppResult<()> {
    let mut store = JournalStore::for_config(config_path, &config.journal.path)?;
    if !store.unlock_available() {
        bail!(
            "age identity not found at {}; encrypted entries cannot be decrypted on this machine",
            store.identity_path().display()
        );
    }
    let passphrase = if store.identity_needs_passphrase()? {
        Some(prompts::prompt_unlock_passphrase()?)
    } else {
        None
    };
    store.unlock(passphrase.as_ref())?;
    let summary = store.decrypt_store(cli_progress())?;
    println!(
        "Decrypted journal store at {}",
        config.journal.path.display()
    );
    if let Some(backup) = summary.backup_path {
        println!("Backup written to {}", backup.display());
    }
    println!(
        "Disabled age identity at {}",
        summary.disabled_identity_file.display()
    );
    if let Some(trust) = summary.disabled_trust_file {
        println!("Retired device trust pins to {}", trust.display());
    }
    Ok(())
}
