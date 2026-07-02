use chrono::Local;
use clap::{Parser, Subcommand};
use journal::{AppResult, config, crypto, storage, tui};
use nanoid::nanoid;
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[cfg(not(unix))]
use std::io::IsTerminal;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

#[derive(Debug, Parser)]
#[command(name = "journal")]
#[command(about = "Markdown terminal journal")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long, value_name = "NAME")]
    journal: Option<String>,

    #[arg(long, value_name = "NAME")]
    set_default: Option<String>,

    #[command(subcommand)]
    command: Option<CliCommand>,

    #[arg(value_name = "TEXT")]
    body: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Encrypt,
    Decrypt,
}

fn main() -> AppResult<()> {
    let cli = Cli::parse();

    if let Some(command) = &cli.command {
        return handle_command(&cli, command);
    }

    if let Some(journal) = cli.set_default.as_deref() {
        return set_default_journal(&cli, journal);
    }

    let stdin_is_pipe = stdin_has_command_input();
    if !cli.body.is_empty() || stdin_is_pipe {
        return create_entry_from_command(cli, stdin_is_pipe);
    }
    if cli.journal.is_some() {
        return Err("--journal requires entry text or piped stdin".into());
    }

    let (config_path, config) = config::load_or_setup_with_path(cli.config.as_deref())?;
    storage::ensure_workspace(&config.journal_root)?;

    let encryption_paths = crypto::EncryptionPaths::for_config(&config_path, &config.journal_root)?;
    tui::run(config, encryption_paths)
}

fn set_default_journal(cli: &Cli, journal: &str) -> AppResult<()> {
    if !cli.body.is_empty() {
        return Err("--set-default cannot be used with entry text".into());
    }
    if cli.journal.is_some() {
        return Err("--set-default cannot be used with --journal".into());
    }

    let (path, mut config) = config::load_existing(cli.config.as_deref())?;
    validate_existing_journal(&config.journal_root, journal)?;
    config.default_journal = Some(journal.to_string());
    config::save_config(&path, &config)?;
    println!("Default journal set to {journal}");
    Ok(())
}

fn create_entry_from_command(cli: Cli, stdin_is_pipe: bool) -> AppResult<()> {
    let body_from_args = !cli.body.is_empty();
    if body_from_args && stdin_is_pipe {
        return Err("entry text cannot be combined with piped stdin".into());
    }

    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let journal = cli
        .journal
        .as_deref()
        .or(config.default_journal.as_deref())
        .ok_or(
            "no journal specified; pass --journal or set one with `journal --set-default <name>`",
        )?;
    validate_existing_journal(&config.journal_root, journal)?;

    let body = if body_from_args {
        cli.body.join(" ")
    } else {
        let mut body = String::new();
        io::stdin().read_to_string(&mut body)?;
        body
    };

    let paths = crypto::EncryptionPaths::for_config(&config_path, &config.journal_root)?;
    let path = if crypto::should_encrypt(&paths) {
        storage::create_encrypted_entry_with_body(&config.journal_root, journal, &body, &paths)?
    } else {
        storage::create_entry_with_body(&config.journal_root, journal, &body)?
    };
    println!("{}", path.display());
    Ok(())
}

fn handle_command(cli: &Cli, command: &CliCommand) -> AppResult<()> {
    validate_no_entry_args(cli)?;
    match command {
        CliCommand::Encrypt => encrypt_workspace(cli),
        CliCommand::Decrypt => decrypt_workspace(cli),
    }
}

fn validate_no_entry_args(cli: &Cli) -> AppResult<()> {
    if !cli.body.is_empty() {
        return Err("command cannot be used with entry text".into());
    }
    if cli.journal.is_some() {
        return Err("command cannot be used with --journal".into());
    }
    if cli.set_default.is_some() {
        return Err("command cannot be used with --set-default".into());
    }
    Ok(())
}

fn encrypt_workspace(cli: &Cli) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let paths = crypto::EncryptionPaths::for_config(&config_path, &config.journal_root)?;
    let recipient = if crypto::should_encrypt(&paths) {
        crypto::public_recipient(&paths)?
    } else if workspace_has_encrypted_entry_files(&config.journal_root)? {
        return Err(format!(
            "encrypted entries already exist but recipients file is missing at {}; cannot safely continue encryption",
            paths.recipients_file.display()
        )
        .into());
    } else {
        println!("No journal encryption identity configured; generating an age identity.");
        crypto::generate_identity_store_interactive(&paths)?
    };

    migrate_workspace(
        &config.journal_root,
        MigrationMode::Encrypt { paths: &paths },
    )?;
    println!(
        "Encrypted journal workspace at {}",
        config.journal_root.display()
    );
    println!(
        "Encryption recipient: {recipient}. Age identity: {}. Back it up; without it encrypted journal files cannot be decrypted.",
        paths.identity_file.display()
    );
    Ok(())
}

fn decrypt_workspace(cli: &Cli) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let paths = crypto::EncryptionPaths::for_config(&config_path, &config.journal_root)?;
    if !crypto::can_decrypt(&paths) {
        return Err(format!(
            "age identity not found at {}; encrypted entries cannot be decrypted on this machine",
            paths.identity_file.display()
        )
        .into());
    }
    let identity = crypto::prompt_unlock_identity(&paths)?;
    migrate_workspace(
        &config.journal_root,
        MigrationMode::Decrypt {
            identity: &identity,
        },
    )?;
    if paths.recipients_file.exists() {
        fs::remove_file(&paths.recipients_file)?;
    }
    let disabled_identity = disable_identity_file(&paths)?;
    println!(
        "Decrypted journal workspace at {}",
        config.journal_root.display()
    );
    println!("Disabled age identity at {}", disabled_identity.display());
    Ok(())
}

enum MigrationMode<'a> {
    Encrypt {
        paths: &'a crypto::EncryptionPaths,
    },
    Decrypt {
        identity: &'a crypto::UnlockedIdentity,
    },
}

fn migrate_workspace(root: &Path, mode: MigrationMode<'_>) -> AppResult<()> {
    let files = migration_files(root, &mode)?;
    if files.is_empty() {
        return Ok(());
    }
    ensure_no_migration_collisions(&files, &mode)?;
    let backup = backup_workspace(root)?;

    let result = (|| -> AppResult<()> {
        for source in files {
            match mode {
                MigrationMode::Encrypt { paths } => encrypt_plain_entry(&source, paths)?,
                MigrationMode::Decrypt { identity } => decrypt_encrypted_entry(&source, identity)?,
            }
        }
        Ok(())
    })();

    if let Err(error) = result {
        eprintln!(
            "Migration failed; plaintext backup remains at {}",
            backup.display()
        );
        return Err(error);
    }

    if matches!(mode, MigrationMode::Encrypt { .. }) {
        fs::remove_dir_all(&backup)?;
    } else {
        println!("Backup written to {}", backup.display());
    }

    Ok(())
}

fn migration_files(root: &Path, mode: &MigrationMode<'_>) -> AppResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_workspace_files_including_trash(root, &mut |path| {
        let matches = match mode {
            MigrationMode::Encrypt { .. } => storage::is_plain_entry_file(path),
            MigrationMode::Decrypt { .. } => storage::is_encrypted_entry_file(path),
        };
        if matches {
            files.push(path.to_path_buf());
        }
        Ok(())
    })?;
    files.sort();
    Ok(files)
}

fn workspace_has_encrypted_entry_files(root: &Path) -> AppResult<bool> {
    let mut has_match = false;
    collect_workspace_files_including_trash(root, &mut |path| {
        if storage::is_encrypted_entry_file(path) {
            has_match = true;
        }
        Ok(())
    })?;
    Ok(has_match)
}

fn collect_workspace_files_including_trash(
    dir: &Path,
    visit: &mut impl FnMut(&Path) -> AppResult<()>,
) -> AppResult<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_workspace_files_including_trash(&path, visit)?;
            continue;
        }
        visit(&path)?;
    }

    Ok(())
}

fn ensure_no_migration_collisions(files: &[PathBuf], mode: &MigrationMode<'_>) -> AppResult<()> {
    for source in files {
        let target = migration_target(source, mode)?;
        if target.exists() {
            return Err(format!(
                "cannot migrate {}; target already exists: {}",
                source.display(),
                target.display()
            )
            .into());
        }
    }
    Ok(())
}

fn encrypt_plain_entry(path: &Path, paths: &crypto::EncryptionPaths) -> AppResult<()> {
    let target = path.with_extension("md.age");
    let temp = unique_temp_path("tmp.age")?;
    crypto::encrypt_file(paths, path, &temp)?;
    fs::rename(&temp, &target)?;
    fs::remove_file(path)?;
    Ok(())
}

fn decrypt_encrypted_entry(path: &Path, identity: &crypto::UnlockedIdentity) -> AppResult<()> {
    let target = decrypted_entry_path(path)?;
    let temp = unique_temp_path("tmp.md")?;
    crypto::decrypt_file(identity, path, &temp)?;
    let decrypted = fs::read_to_string(&temp)?;
    if decrypted.is_empty() {
        let _ = fs::remove_file(&temp);
        return Err(format!("decrypted entry is empty: {}", path.display()).into());
    }
    fs::rename(&temp, &target)?;
    fs::remove_file(path)?;
    Ok(())
}

fn migration_target(path: &Path, mode: &MigrationMode<'_>) -> AppResult<PathBuf> {
    match mode {
        MigrationMode::Encrypt { .. } => Ok(path.with_extension("md.age")),
        MigrationMode::Decrypt { .. } => decrypted_entry_path(path),
    }
}

fn decrypted_entry_path(path: &Path) -> AppResult<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("encrypted entry path has no UTF-8 file name")?;
    let plain_name = name
        .strip_suffix(".md.age")
        .ok_or("encrypted entry path does not end in .md.age")?;
    Ok(path.with_file_name(format!("{plain_name}.md")))
}

fn backup_workspace(root: &Path) -> AppResult<PathBuf> {
    let backup = backup_path(root);
    copy_dir_all(root, &backup)?;
    Ok(backup)
}

fn backup_path(root: &Path) -> PathBuf {
    let timestamp = Local::now().format("%Y%m%d%H%M%S%f");
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("journal");
    root.with_file_name(format!("{name}.backup-{timestamp}"))
}

fn copy_dir_all(source: &Path, target: &Path) -> AppResult<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn disable_identity_file(paths: &crypto::EncryptionPaths) -> AppResult<PathBuf> {
    let target = disabled_identity_path(&paths.identity_file);
    fs::rename(&paths.identity_file, &target)?;
    Ok(target)
}

fn disabled_identity_path(identity_file: &Path) -> PathBuf {
    let timestamp = Local::now().format("%Y%m%d%H%M%S");
    disabled_identity_path_for_timestamp(identity_file, &timestamp.to_string())
}

fn disabled_identity_path_for_timestamp(identity_file: &Path, timestamp: &str) -> PathBuf {
    let parent = identity_file.parent().unwrap_or_else(|| Path::new(""));
    let base = parent.join(format!("identity.disabled-{timestamp}.age"));
    if !base.exists() {
        return base;
    }

    for _ in 0..32 {
        let candidate = parent.join(format!("identity.disabled-{timestamp}-{}.age", nanoid!(6)));
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(format!(
        "identity.disabled-{timestamp}-{}.age",
        Local::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

fn unique_temp_path(suffix: &str) -> AppResult<PathBuf> {
    Ok(std::env::temp_dir().join(format!(
        ".journal-{}-{}.{}",
        std::process::id(),
        Local::now().timestamp_nanos_opt().unwrap_or_default(),
        suffix
    )))
}

#[cfg(unix)]
fn stdin_has_command_input() -> bool {
    std::fs::metadata("/dev/stdin")
        .map(|metadata| {
            let file_type = metadata.file_type();
            file_type.is_fifo() || file_type.is_socket() || file_type.is_file()
        })
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn stdin_has_command_input() -> bool {
    !io::stdin().is_terminal()
}

fn validate_existing_journal(root: &std::path::Path, journal: &str) -> AppResult<()> {
    let journal = storage::validate_journal_name(journal)?;
    let path = root.join(&journal);
    if !path.is_dir() {
        return Err(format!("journal '{journal}' does not exist").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn disabled_identity_path_uses_timestamped_age_filename() {
        let dir = tempdir().unwrap();
        let identity = dir.path().join("identity.age");

        let disabled = disabled_identity_path_for_timestamp(&identity, "20260702123456");

        assert_eq!(
            disabled,
            dir.path().join("identity.disabled-20260702123456.age")
        );
    }

    #[test]
    fn disabled_identity_path_adds_suffix_when_target_exists() {
        let dir = tempdir().unwrap();
        let identity = dir.path().join("identity.age");
        let base = dir.path().join("identity.disabled-20260702123456.age");
        fs::write(&base, "existing").unwrap();

        let disabled = disabled_identity_path_for_timestamp(&identity, "20260702123456");

        assert_ne!(disabled, base);
        let name = disabled.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("identity.disabled-20260702123456-"));
        assert!(name.ends_with(".age"));
    }

    #[test]
    fn disable_identity_file_renames_active_identity() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let root = dir.path().join("journals");
        let paths = crypto::EncryptionPaths::for_config(&config, &root).unwrap();
        fs::write(&paths.identity_file, "identity").unwrap();

        let disabled = disable_identity_file(&paths).unwrap();

        assert!(!paths.identity_file.exists());
        assert_eq!(fs::read_to_string(disabled).unwrap(), "identity");
    }

    #[test]
    fn encrypted_entries_in_trash_are_detected_for_migration_safety() {
        let dir = tempdir().unwrap();
        let encrypted_trash = dir
            .path()
            .join("work")
            .join(".trash")
            .join("2026")
            .join("07")
            .join("02")
            .join("old.md.age");
        fs::create_dir_all(encrypted_trash.parent().unwrap()).unwrap();
        fs::write(encrypted_trash, "ciphertext").unwrap();

        assert!(workspace_has_encrypted_entry_files(dir.path()).unwrap());
    }
}
