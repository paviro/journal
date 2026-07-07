use crate::AppResult;
use journal_storage::JournalStore;
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

pub fn load_or_setup_with_path(path_override: Option<&Path>) -> AppResult<(PathBuf, Config)> {
    let config_path = config_path(path_override)?;

    if config_path.exists() {
        let config = load_config(&config_path)?;
        JournalStore::for_config(&config_path, &config.journal_root)?.ensure()?;
        return Ok((config_path, config));
    }

    let config = interactive_setup(&config_path)?;
    Ok((config_path, config))
}

pub fn load_existing(path_override: Option<&Path>) -> AppResult<(PathBuf, Config)> {
    let config_path = config_path(path_override)?;
    if !config_path.exists() {
        return Err(format!(
            "config file not found at {}; run `journal` once to set it up or pass --config",
            config_path.display()
        )
        .into());
    }

    let config = load_config(&config_path)?;
    JournalStore::for_config(&config_path, &config.journal_root)?.ensure()?;
    Ok((config_path, config))
}

fn config_path(path_override: Option<&Path>) -> AppResult<PathBuf> {
    match path_override {
        Some(path) => Ok(path.to_path_buf()),
        None => default_config_path(),
    }
}

fn interactive_setup(config_path: &Path) -> AppResult<Config> {
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

    write!(stdout, "Enable encryption? [y/N]: ")?;
    stdout.flush()?;
    let mut encryption_input = String::new();
    io::stdin().read_line(&mut encryption_input)?;

    let config = Config::new(journal_root, editor);
    let encryption_enabled = matches!(encryption_input.trim(), "y" | "Y" | "yes" | "YES" | "Yes");
    if encryption_enabled {
        writeln!(
            stdout,
            "Generating a passphrase-protected journal age identity."
        )?;
        let store = JournalStore::for_config(config_path, &config.journal_root)?;
        let passphrase = crate::migrate::prompt_new_passphrase()?;
        store.initialize_encryption(&passphrase)?;
        writeln!(
            stdout,
            "Age identity: {}. Back it up; without it encrypted journal files cannot be decrypted.",
            store.paths().identity_file.display()
        )?;
    }

    save_config(config_path, &config)?;
    JournalStore::for_config(config_path, &config.journal_root)?.ensure()?;

    // The TUI enters the alternate screen and wipes this output, so hold here
    // until the user has read the identity-backup warning before opening it.
    if encryption_enabled {
        write!(stdout, "\nPress Enter to open your journal…")?;
        stdout.flush()?;
        let mut ack = String::new();
        io::stdin().read_line(&mut ack)?;
    }

    Ok(config)
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
