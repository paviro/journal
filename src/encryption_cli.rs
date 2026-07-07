use crate::{AppResult, config::Config, prompts};
use indicatif::{ProgressBar, ProgressStyle};
use journal_storage::JournalStore;
use std::path::Path;

/// A progress sink for CLI migrations that drives an `indicatif` bar. A fresh
/// bar is created at the start of each pass (a `(0, total)` tick) — so a
/// two-pass operation like rotation shows a bar per pass — and cleared when the
/// pass completes.
pub fn cli_progress() -> impl FnMut(usize, usize) {
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

pub fn encrypt_store(
    config_path: &Path,
    config: &Config,
    device_name: Option<&str>,
    no_passphrase: bool,
) -> AppResult<()> {
    let store = JournalStore::for_config(config_path, &config.journal_root)?;
    let mut bootstrapped_without_passphrase = false;
    let recipient = if store.encryption_enabled() {
        if !store.unlock_available() {
            return Err(format!(
                "this journal is already encrypted for other devices, but this one has no key at {}; run `{}` to request access instead",
                store.paths().keys.identity_file.display(),
                crate::ENROLL_CMD,
            )
            .into());
        }
        store.public_recipient()?
    } else if store.has_encrypted_entries()? {
        // Encrypted entries but no roster to encrypt more against — reuse the
        // storage layer's own message rather than restating it here.
        return Err(journal_storage::EncryptionError::RecipientsMissing {
            path: store.paths().keys.devices_file.clone(),
        }
        .to_string()
        .into());
    } else {
        println!("No journal encryption identity configured; generating an age identity.");
        let (name, passphrase) = prompts::resolve_new_identity_options(device_name, no_passphrase)?;
        bootstrapped_without_passphrase = passphrase.is_none();
        store.initialize_encryption(&name, passphrase.as_ref())?
    };

    store.encrypt_store(cli_progress())?;
    println!(
        "Encrypted journal store at {}",
        config.journal_root.display()
    );
    println!(
        "Encryption recipient: {recipient}. Identity file: {}. Back it up; without it encrypted journal files cannot be decrypted.",
        store.paths().keys.identity_file.display()
    );
    if bootstrapped_without_passphrase {
        println!("This key has no passphrase — keep this device and its backups secure.");
    }
    Ok(())
}

pub fn decrypt_store(config_path: &Path, config: &Config) -> AppResult<()> {
    let mut store = JournalStore::for_config(config_path, &config.journal_root)?;
    if !store.unlock_available() {
        return Err(format!(
            "age identity not found at {}; encrypted entries cannot be decrypted on this machine",
            store.paths().keys.identity_file.display()
        )
        .into());
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
        config.journal_root.display()
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
