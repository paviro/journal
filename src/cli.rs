use crate::{AppResult, config, crypto, feelings, migrate, storage, tui};
use clap::{Parser, Subcommand};
use std::{
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

    #[arg(long, value_name = "TAG")]
    tag: Vec<String>,

    #[arg(long, value_name = "LABEL")]
    feeling: Vec<String>,

    #[arg(long, value_name = "SCORE", allow_hyphen_values = true)]
    mood: Option<i8>,

    #[command(subcommand)]
    command: Option<CliCommand>,

    #[arg(value_name = "TEXT")]
    body: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Set the default journal for new entries
    Default {
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Encrypt every plaintext entry in the workspace
    Encrypt,
    /// Decrypt every encrypted entry in the workspace
    Decrypt,
}

pub fn run() -> AppResult<()> {
    let cli = Cli::parse();

    if let Some(command) = &cli.command {
        return handle_command(&cli, command);
    }

    let stdin_is_pipe = stdin_has_command_input();
    if !cli.body.is_empty() || stdin_is_pipe {
        return create_entry_from_command(cli, stdin_is_pipe);
    }
    if cli.journal.is_some() || !cli.tag.is_empty() || !cli.feeling.is_empty() || cli.mood.is_some()
    {
        return Err(
            "--journal, --tag, --feeling, and --mood require entry text or piped stdin".into(),
        );
    }

    let (config_path, config) = config::load_or_setup_with_path(cli.config.as_deref())?;
    storage::ensure_workspace(&config.journal_root)?;

    let encryption_paths = crypto::EncryptionPaths::for_config(&config_path, &config.journal_root)?;
    tui::run(config, encryption_paths)
}

fn handle_command(cli: &Cli, command: &CliCommand) -> AppResult<()> {
    validate_no_entry_args(cli)?;
    match command {
        CliCommand::Default { name } => set_default_journal(cli, name),
        CliCommand::Encrypt => {
            let (config_path, config) = config::load_existing(cli.config.as_deref())?;
            migrate::encrypt_workspace(&config_path, &config)
        }
        CliCommand::Decrypt => {
            let (config_path, config) = config::load_existing(cli.config.as_deref())?;
            migrate::decrypt_workspace(&config_path, &config)
        }
    }
}

fn validate_no_entry_args(cli: &Cli) -> AppResult<()> {
    if !cli.body.is_empty() {
        return Err("command cannot be used with entry text".into());
    }
    if cli.journal.is_some() {
        return Err("command cannot be used with --journal".into());
    }
    if !cli.tag.is_empty() {
        return Err("command cannot be used with --tag".into());
    }
    if !cli.feeling.is_empty() {
        return Err("command cannot be used with --feeling".into());
    }
    if cli.mood.is_some() {
        return Err("command cannot be used with --mood".into());
    }
    Ok(())
}

fn set_default_journal(cli: &Cli, journal: &str) -> AppResult<()> {
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
        .ok_or("no journal specified; pass --journal or set one with `journal default <name>`")?;
    validate_existing_journal(&config.journal_root, journal)?;
    let tags: Vec<String> = cli
        .tag
        .iter()
        .flat_map(|t| t.split(','))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    let feelings = feelings::validate_feelings(
        cli.feeling
            .iter()
            .flat_map(|f| f.split(','))
            .map(str::trim)
            .filter(|f| !f.is_empty()),
    )?;
    let mood = if let Some(score) = cli.mood {
        if !(-5..=5).contains(&score) {
            return Err(format!("--mood must be between -5 and +5, got {score}").into());
        }
        Some(score)
    } else {
        None
    };

    let body = if body_from_args {
        cli.body.join(" ")
    } else {
        let mut body = String::new();
        io::stdin().read_to_string(&mut body)?;
        body
    };

    let paths = crypto::EncryptionPaths::for_config(&config_path, &config.journal_root)?;
    let path = if crypto::should_encrypt(&paths) {
        storage::create_encrypted_entry_with_body_and_feelings(
            &config.journal_root,
            journal,
            &body,
            &tags,
            &feelings,
            mood,
            &paths,
        )?
    } else {
        storage::create_entry_with_body_and_feelings(
            &config.journal_root,
            journal,
            &body,
            &tags,
            &feelings,
            mood,
        )?
    };
    println!("{}", path.display());
    Ok(())
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

fn validate_existing_journal(root: &Path, journal: &str) -> AppResult<()> {
    let journal = storage::validate_journal_name(journal)?;
    let path = root.join(&journal);
    if !path.is_dir() {
        return Err(format!("journal '{journal}' does not exist").into());
    }
    Ok(())
}
