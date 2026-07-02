use crate::AppResult;
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
}

impl Config {
    pub fn new(journal_root: PathBuf, editor: impl Into<String>) -> Self {
        Self {
            journal_root: expand_tilde(journal_root),
            editor: editor.into(),
            default_journal: None,
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

pub fn load_or_setup(path_override: Option<&Path>) -> AppResult<Config> {
    Ok(load_or_setup_with_path(path_override)?.1)
}

pub fn load_or_setup_with_path(path_override: Option<&Path>) -> AppResult<(PathBuf, Config)> {
    let config_path = config_path(path_override)?;

    if config_path.exists() {
        let config = load_config(&config_path)?;
        crate::storage::ensure_workspace(&config.journal_root)?;
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
    crate::storage::ensure_workspace(&config.journal_root)?;
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

    write!(stdout, "Enable age encryption? [y/N]: ")?;
    stdout.flush()?;
    let mut encryption_input = String::new();
    io::stdin().read_line(&mut encryption_input)?;

    let config = Config::new(journal_root, editor);
    if matches!(encryption_input.trim(), "y" | "Y" | "yes" | "YES" | "Yes") {
        writeln!(
            stdout,
            "Generating a passphrase-protected journal age identity."
        )?;
        let paths = crate::crypto::EncryptionPaths::for_config(config_path, &config.journal_root)?;
        crate::crypto::generate_identity_store_interactive(&paths)?;
        writeln!(
            stdout,
            "Age identity: {}. Back it up; without it encrypted journal files cannot be decrypted.",
            paths.identity_file.display()
        )?;
    }

    save_config(config_path, &config)?;
    crate::storage::ensure_workspace(&config.journal_root)?;
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
    fn explicit_missing_config_runs_setup_path_behavior_when_saved_directly() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let config = Config::new(dir.path().join("root"), "vim");

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn save_and_load_config_preserves_default_journal() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::new(dir.path().join("root"), "vim");
        config.default_journal = Some("work".to_string());

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded.default_journal.as_deref(), Some("work"));
    }

    #[test]
    fn saved_config_omits_encryption_state() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config::new(dir.path().join("root"), "vim");

        save_config(&path, &config).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("encryption"));
    }
}
