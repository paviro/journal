use crate::{AppResult, config, editor, migrate, tui};
use clap::{Args, Parser, Subcommand};
use journal_core::feelings;
use journal_storage::{EntryMetadata, JournalStore};
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

    #[arg(long, value_name = "NAME", hide = true)]
    journal: Option<String>,

    #[arg(long, value_name = "TAG", hide = true)]
    tag: Vec<String>,

    #[arg(long, value_name = "NAME", hide = true)]
    person: Vec<String>,

    #[arg(long, value_name = "ACTIVITY", hide = true)]
    activity: Vec<String>,

    #[arg(long, value_name = "LABEL", hide = true)]
    feeling: Vec<String>,

    #[arg(long, value_name = "SCORE", allow_hyphen_values = true, hide = true)]
    mood: Option<i8>,

    #[command(subcommand)]
    command: Option<CliCommand>,

    #[arg(value_name = "TEXT")]
    body: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Create a journal entry from text, stdin, or the configured editor
    Log(LogArgs),
    /// Set the default journal for new entries
    Use {
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Encrypt every plaintext entry in the store
    Encrypt,
    /// Decrypt every encrypted entry in the store
    Decrypt,
}

#[derive(Debug, Args)]
struct LogArgs {
    #[arg(long, value_name = "NAME")]
    journal: Option<String>,

    #[arg(long, value_name = "TAG")]
    tag: Vec<String>,

    #[arg(long, value_name = "NAME")]
    person: Vec<String>,

    #[arg(long, value_name = "ACTIVITY")]
    activity: Vec<String>,

    #[arg(long, value_name = "LABEL")]
    feeling: Vec<String>,

    #[arg(long, value_name = "SCORE", allow_hyphen_values = true)]
    mood: Option<i8>,

    #[arg(value_name = "TEXT")]
    body: Vec<String>,
}

pub fn run() -> AppResult<()> {
    let cli = Cli::parse();
    let stdin_is_pipe = stdin_has_command_input();

    if let Some(command) = &cli.command {
        return handle_command(&cli, command, stdin_is_pipe);
    }

    if !cli.body.is_empty() {
        return Err("entry text requires `journal log`; run `journal log <text>`".into());
    }
    if stdin_is_pipe {
        return Err(
            "piped entry text requires `journal log`; run `journal log` with piped stdin".into(),
        );
    }
    if cli.journal.is_some()
        || !cli.tag.is_empty()
        || !cli.person.is_empty()
        || !cli.activity.is_empty()
        || !cli.feeling.is_empty()
        || cli.mood.is_some()
    {
        return Err(
            "--journal, --tag, --person, --activity, --feeling, and --mood belong to `journal log`"
                .into(),
        );
    }

    let (config_path, config) = config::load_or_setup_with_path(cli.config.as_deref())?;
    let store = JournalStore::for_config(&config_path, &config.journal_root)?;
    store.ensure()?;

    tui::run(config_path, config, store)
}

fn handle_command(cli: &Cli, command: &CliCommand, stdin_is_pipe: bool) -> AppResult<()> {
    match command {
        CliCommand::Log(args) => {
            validate_no_legacy_entry_args(cli)?;
            create_entry_from_log_command(cli, args, stdin_is_pipe)
        }
        CliCommand::Use { name } => {
            validate_no_legacy_entry_args(cli)?;
            set_default_journal(cli, name)
        }
        CliCommand::Encrypt => {
            validate_no_legacy_entry_args(cli)?;
            let (config_path, config) = config::load_existing(cli.config.as_deref())?;
            migrate::encrypt_store(&config_path, &config)
        }
        CliCommand::Decrypt => {
            validate_no_legacy_entry_args(cli)?;
            let (config_path, config) = config::load_existing(cli.config.as_deref())?;
            migrate::decrypt_store(&config_path, &config)
        }
    }
}

fn validate_no_legacy_entry_args(cli: &Cli) -> AppResult<()> {
    if !cli.body.is_empty() {
        return Err("entry text requires `journal log`; run `journal log <text>`".into());
    }
    if cli.journal.is_some() {
        return Err("--journal belongs to `journal log`".into());
    }
    if !cli.tag.is_empty() {
        return Err("--tag belongs to `journal log`".into());
    }
    if !cli.person.is_empty() {
        return Err("--person belongs to `journal log`".into());
    }
    if !cli.activity.is_empty() {
        return Err("--activity belongs to `journal log`".into());
    }
    if !cli.feeling.is_empty() {
        return Err("--feeling belongs to `journal log`".into());
    }
    if cli.mood.is_some() {
        return Err("--mood belongs to `journal log`".into());
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

fn create_entry_from_log_command(cli: &Cli, args: &LogArgs, stdin_is_pipe: bool) -> AppResult<()> {
    let body_from_args = !args.body.is_empty();
    if body_from_args && stdin_is_pipe {
        return Err("entry text cannot be combined with piped stdin".into());
    }

    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let journal = args
        .journal
        .as_deref()
        .or(config.default_journal.as_deref())
        .ok_or("no journal specified; pass --journal or set one with `journal use <name>`")?;
    validate_existing_journal(&config.journal_root, journal)?;
    let tags = comma_separated_values(&args.tag);
    let people = comma_separated_values(&args.person);
    let activities = comma_separated_values(&args.activity);
    let feelings = feelings::validate_feelings(
        args.feeling
            .iter()
            .flat_map(|f| f.split(','))
            .map(str::trim)
            .filter(|f| !f.is_empty()),
    )?;
    let mood = if let Some(score) = args.mood {
        if !(-5..=5).contains(&score) {
            return Err(format!("--mood must be between -5 and +5, got {score}").into());
        }
        Some(score)
    } else {
        None
    };
    let metadata = EntryMetadata {
        tags: &tags,
        people: &people,
        activities: &activities,
        feelings: &feelings,
        mood,
    };

    let store = JournalStore::for_config(&config_path, &config.journal_root)?;
    let path = if body_from_args || stdin_is_pipe {
        let body = if body_from_args {
            args.body.join(" ")
        } else {
            let mut body = String::new();
            io::stdin().read_to_string(&mut body)?;
            body
        };

        Some(store.create_entry_with_body(journal, &body, metadata)?)
    } else {
        let editor_cmd = config.editor.clone();
        store.create_entry_via_editor(journal, metadata, |body| {
            editor::edit_body(&editor_cmd, body)
        })?
    };
    if let Some(path) = path {
        println!("{}", path.display());
    }
    Ok(())
}

fn comma_separated_values(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
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
    let journal = JournalStore::validate_journal_name(journal)?;
    let path = root.join(&journal);
    if !path.is_dir() {
        return Err(format!("journal '{journal}' does not exist").into());
    }
    Ok(())
}
