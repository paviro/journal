use crate::{AppResult, config::Config};
use journal_storage::JournalStore;
use rpassword::prompt_password;
use std::path::Path;

pub fn prompt_new_passphrase() -> AppResult<String> {
    let passphrase = prompt_password("New journal encryption passphrase: ")?;
    if passphrase.is_empty() {
        return Err("encryption passphrase cannot be empty".into());
    }
    let confirm = prompt_password("Confirm journal encryption passphrase: ")?;
    if passphrase != confirm {
        return Err("encryption passphrases did not match".into());
    }
    Ok(passphrase)
}

pub fn prompt_unlock_passphrase() -> AppResult<String> {
    Ok(prompt_password("Journal encryption passphrase: ")?)
}

pub fn encrypt_store(config_path: &Path, config: &Config) -> AppResult<()> {
    let store = JournalStore::for_config(config_path, &config.journal_root)?;
    let recipient = if store.encryption_enabled() {
        store.public_recipient()?
    } else if store.has_encrypted_entries()? {
        return Err(format!(
            "encrypted entries already exist but recipients file is missing at {}; cannot safely continue encryption",
            store.paths().recipients_file.display()
        )
        .into());
    } else {
        println!("No journal encryption identity configured; generating an age identity.");
        let passphrase = prompt_new_passphrase()?;
        store.initialize_encryption(&passphrase)?
    };

    store.encrypt_store()?;
    println!(
        "Encrypted journal store at {}",
        config.journal_root.display()
    );
    println!(
        "Encryption recipient: {recipient}. Age identity: {}. Back it up; without it encrypted journal files cannot be decrypted.",
        store.paths().identity_file.display()
    );
    Ok(())
}

pub fn decrypt_store(config_path: &Path, config: &Config) -> AppResult<()> {
    let mut store = JournalStore::for_config(config_path, &config.journal_root)?;
    if !store.unlock_available() {
        return Err(format!(
            "age identity not found at {}; encrypted entries cannot be decrypted on this machine",
            store.paths().identity_file.display()
        )
        .into());
    }
    let passphrase = prompt_unlock_passphrase()?;
    store.unlock(&passphrase)?;
    let summary = store.decrypt_store()?;
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
    Ok(())
}
