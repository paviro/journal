use crate::AppResult;
use journal_storage::{JournalStore, SecretString};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub journal_root: PathBuf,
    pub editor: String,
    #[serde(default)]
    pub default_journal: Option<String>,
    #[serde(default = "default_true")]
    pub show_hints: bool,
    #[serde(default = "default_true")]
    pub show_journals: bool,
    #[serde(default)]
    pub last_journal: Option<String>,
    #[serde(default = "default_true")]
    pub download_remote_images: bool,
}

fn default_true() -> bool {
    true
}

impl Config {
    pub fn new(journal_root: PathBuf, editor: impl Into<String>) -> Self {
        Self {
            journal_root: expand_tilde(journal_root),
            editor: editor.into(),
            default_journal: None,
            show_hints: true,
            show_journals: true,
            last_journal: None,
            download_remote_images: true,
        }
    }
}

pub fn default_config_path() -> AppResult<PathBuf> {
    if let Ok(config_home) = env::var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home)
            .join("journal")
            .join("config.toml"));
    }

    let home = dirs::home_dir().ok_or("could not determine home directory")?;
    Ok(home.join(".config").join("journal").join("config.toml"))
}

pub fn load_config(path: &Path) -> AppResult<Config> {
    let text = fs::read_to_string(path)?;
    let mut config: Config = toml::from_str(&text)?;
    config.journal_root = expand_tilde(config.journal_root);
    Ok(config)
}

pub fn save_config(path: &Path, config: &Config) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let text = toml::to_string_pretty(config)?;
    fs::write(path, text)?;
    Ok(())
}

/// What `load_or_setup_with_path` resolved: a store ready to open in the TUI. An
/// encrypted store this device can't yet read is opened too — the TUI shows the
/// enroll/awaiting notice rather than the CLI printing it.
pub struct Startup {
    pub config_path: PathBuf,
    pub config: Config,
    pub store: Box<JournalStore>,
}

pub fn load_or_setup_with_path(path_override: Option<&Path>) -> AppResult<Startup> {
    let config_path = config_path(path_override)?;

    // An encrypted store this device can't yet read (no key, awaiting approval, or
    // revoked) is still opened: the TUI shows the enroll/awaiting notice instead
    // of the CLI printing a hint, so every unreadable-store case looks the same.
    // Reconciling a remote encryption *disable* is likewise deferred to the TUI,
    // which must run it before probing for a lock.
    let (config, store) = if config_path.exists() {
        let config = load_config(&config_path)?;
        let store = JournalStore::for_config(&config_path, &config.journal_root)?;
        store.ensure()?;
        (config, store)
    } else {
        interactive_setup(&config_path)?
    };

    Ok(Startup {
        config_path,
        config,
        store: Box::new(store),
    })
}

pub fn load_existing(path_override: Option<&Path>) -> AppResult<(PathBuf, Config)> {
    let config_path = config_path(path_override)?;
    if !config_path.exists() {
        return Err(format!(
            "config file not found at {}; run `journal` once to set it up or pass --config <DIR>",
            config_path.display()
        )
        .into());
    }

    let config = load_config(&config_path)?;
    let store = JournalStore::for_config(&config_path, &config.journal_root)?;
    store.ensure()?;
    if store.reconcile_disabled_encryption()? {
        eprintln!(
            "Note: encryption was disabled on another device; retired this device's key and trust pins."
        );
    }
    Ok((config_path, config))
}

/// Resolve the config *file* from an optional config-directory override. The
/// override names the directory that holds `config.toml` alongside this device's
/// encryption key; without one we fall back to the XDG default.
fn config_path(path_override: Option<&Path>) -> AppResult<PathBuf> {
    match path_override {
        Some(dir) => {
            // `--config` names the directory, not the file. Passing a file (a
            // stale `.../config.toml`) would silently nest into
            // `.../config.toml/config.toml` and trigger a bogus first-run setup.
            if dir.is_file() || dir.file_name() == Some(std::ffi::OsStr::new("config.toml")) {
                return Err(format!(
                    "--config takes a directory, not a file; pass {} instead",
                    dir.parent().unwrap_or(dir).display()
                )
                .into());
            }
            Ok(dir.join("config.toml"))
        }
        None => default_config_path(),
    }
}

fn interactive_setup(config_path: &Path) -> AppResult<(Config, JournalStore)> {
    let mut stdout = io::stdout();
    let default_root = dirs::home_dir()
        .map(|home| home.join("Journals"))
        .unwrap_or_else(|| PathBuf::from("Journals"));

    writeln!(stdout, "Journal first-run setup")?;
    write!(
        stdout,
        "Journal root [{}]: ",
        default_root.to_string_lossy()
    )?;
    stdout.flush()?;

    let mut root_input = String::new();
    io::stdin().read_line(&mut root_input)?;
    let journal_root = if root_input.trim().is_empty() {
        default_root
    } else {
        PathBuf::from(root_input.trim())
    };

    write!(stdout, "Editor [nano]: ")?;
    stdout.flush()?;
    let mut editor_input = String::new();
    io::stdin().read_line(&mut editor_input)?;
    let editor = if editor_input.trim().is_empty() {
        "nano".to_string()
    } else {
        editor_input.trim().to_string()
    };

    let config = Config::new(journal_root, editor);
    let store = JournalStore::for_config(config_path, &config.journal_root)?;
    store.ensure()?;

    if should_offer_encryption(&store)? {
        offer_encryption(&mut stdout, &store)?;
    } else if !store.encryption_enabled() {
        // An existing plaintext journal is registered as-is; encryption stays a
        // deliberate later step rather than a first-run prompt.
        writeln!(
            stdout,
            "Using existing journal at {}. Encryption is off; run `journal encryption enable` to turn it on.",
            config.journal_root.display()
        )?;
    }

    save_config(config_path, &config)?;
    Ok((config, store))
}

/// First-run offers to enable encryption only for a brand-new, empty root — never
/// for a journal that already has entries or is already encrypted. Those are just
/// registered; encryption is managed with the `journal encryption …` commands.
fn should_offer_encryption(store: &JournalStore) -> AppResult<bool> {
    Ok(!store.encryption_enabled() && store.list_journals()?.is_empty())
}

/// Prompt to enable encryption on a fresh store and, if accepted, generate this
/// device's identity. Holds for a keypress afterward because the TUI's alternate
/// screen would otherwise wipe the identity-backup warning.
fn offer_encryption(stdout: &mut impl Write, store: &JournalStore) -> AppResult<()> {
    write!(stdout, "Enable encryption? [y/N]: ")?;
    stdout.flush()?;
    let mut encryption_input = String::new();
    io::stdin().read_line(&mut encryption_input)?;
    if !matches!(encryption_input.trim(), "y" | "Y" | "yes" | "YES" | "Yes") {
        return Ok(());
    }

    let (device_name, passphrase) = resolve_new_identity_options(None, false)?;
    store.initialize_encryption(&device_name, passphrase.as_ref())?;
    writeln!(
        stdout,
        "Age identity: {}. Back it up; without it encrypted journal files cannot be decrypted.",
        store.paths().keys.identity_file.display()
    )?;
    if passphrase.is_none() {
        writeln!(
            stdout,
            "This key has no passphrase, so anyone with this file can read the journal — keep the device and its backups secure."
        )?;
    }

    write!(stdout, "\nPress Enter to open your journal…")?;
    stdout.flush()?;
    let mut ack = String::new();
    io::stdin().read_line(&mut ack)?;
    Ok(())
}

/// Prompt for this device's name (used to label its key), defaulting to the
/// hostname.
fn prompt_device_name(stdout: &mut impl Write) -> AppResult<String> {
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
fn prompt_passphrase_choice(stdout: &mut impl Write) -> AppResult<Option<SecretString>> {
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
        Ok(Some(crate::migrate::prompt_new_passphrase()?))
    }
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

pub fn expand_tilde(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return dirs::home_dir().unwrap_or(path);
    }

    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }

    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store_in(dir: &Path) -> JournalStore {
        let store = JournalStore::for_config(&dir.join("config.toml"), &dir.join("journals"))
            .unwrap();
        store.ensure().unwrap();
        store
    }

    #[test]
    fn offers_encryption_only_for_an_empty_new_root() {
        let dir = tempdir().unwrap();
        let store = store_in(dir.path());
        assert!(should_offer_encryption(&store).unwrap());
    }

    #[test]
    fn skips_encryption_prompt_for_an_existing_plaintext_journal() {
        let dir = tempdir().unwrap();
        let store = store_in(dir.path());
        store.create_journal("work").unwrap();
        assert!(!should_offer_encryption(&store).unwrap());
    }

    #[test]
    fn skips_encryption_prompt_for_an_already_encrypted_store() {
        let dir = tempdir().unwrap();
        let store = store_in(dir.path());
        store.initialize_encryption("laptop", None).unwrap();
        assert!(!should_offer_encryption(&store).unwrap());
    }

    #[test]
    fn config_path_rejects_a_file_argument() {
        let err = config_path(Some(Path::new("/some/dir/config.toml"))).unwrap_err();
        assert!(err.to_string().contains("takes a directory"));
    }

    #[test]
    fn save_and_load_config_expands_tilde_root() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "journal_root = \"~/Journals\"\neditor = \"nano\"\n").unwrap();

        let config = load_config(&path).unwrap();

        assert!(config.journal_root.ends_with("Journals"));
        assert_eq!(config.editor, "nano");
        assert_eq!(config.default_journal, None);
    }

    #[test]
    fn save_and_load_config_round_trips_all_fields() {
        let dir = tempdir().unwrap();
        // A nested path also exercises that save creates missing parent dirs.
        let path = dir.path().join("nested").join("config.toml");
        let mut config = Config::new(dir.path().join("root"), "vim");
        config.default_journal = Some("work".to_string());
        config.show_journals = false;
        config.last_journal = Some("home".to_string());
        config.download_remote_images = false;

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn missing_optional_fields_use_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "journal_root = \"~/Journals\"\neditor = \"nano\"\n").unwrap();

        let config = load_config(&path).unwrap();

        assert!(config.show_journals);
        assert_eq!(config.last_journal, None);
        assert!(config.download_remote_images);
    }
}
