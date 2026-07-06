use crate::{AppResult, config, editor, migrate, tui};
use clap::{Args, Parser, Subcommand};
use journal_core::feelings;
use journal_storage::{JournalStore, MOOD_RANGE, Metadata};
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
    /// Import entries from another journaling app
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },
}

#[derive(Debug, Subcommand)]
enum ImportSource {
    /// Import a Day One JSON export (with photos)
    Dayone(DayoneArgs),
}

#[derive(Debug, Args)]
struct DayoneArgs {
    /// Path to the Day One export `.json` file
    #[arg(value_name = "PATH")]
    path: PathBuf,

    /// Journal to import into (created if missing); defaults to the configured journal
    #[arg(long, value_name = "NAME")]
    journal: Option<String>,

    /// Download remote http(s) image links found in entry bodies. Off by
    /// default; when on, unreachable hosts are detected once and skipped rather
    /// than retried for every link. Skipped links are left in place in the body.
    #[arg(long)]
    download_images: bool,
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

    validate_no_legacy_entry_args(&cli)?;
    if stdin_is_pipe {
        return Err(
            "piped entry text requires `journal log`; run `journal log` with piped stdin".into(),
        );
    }

    let (config_path, config) = config::load_or_setup_with_path(cli.config.as_deref())?;
    let store = JournalStore::for_config(&config_path, &config.journal_root)?;
    store.ensure()?;

    tui::run(config_path, config, store)
}

fn handle_command(cli: &Cli, command: &CliCommand, stdin_is_pipe: bool) -> AppResult<()> {
    validate_no_legacy_entry_args(cli)?;
    match command {
        CliCommand::Log(args) => create_entry_from_log_command(cli, args, stdin_is_pipe),
        CliCommand::Use { name } => set_default_journal(cli, name),
        CliCommand::Encrypt => {
            let (config_path, config) = config::load_existing(cli.config.as_deref())?;
            migrate::encrypt_store(&config_path, &config)
        }
        CliCommand::Decrypt => {
            let (config_path, config) = config::load_existing(cli.config.as_deref())?;
            migrate::decrypt_store(&config_path, &config)
        }
        CliCommand::Import { source } => match source {
            ImportSource::Dayone(args) => import_dayone_command(cli, args),
        },
    }
}

fn import_dayone_command(cli: &Cli, args: &DayoneArgs) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let journal = args
        .journal
        .as_deref()
        .or(config.default_journal.as_deref())
        .ok_or("no journal specified; pass --journal or set one with `journal use <name>`")?;
    // Validate the name only — the importer creates the journal if it's missing.
    let journal = JournalStore::validate_journal_name(journal)?;

    let store = JournalStore::for_config(&config_path, &config.journal_root)?;
    store.ensure()?;

    let report = journal_import::import_dayone(&store, &journal, &args.path, args.download_images)?;

    println!(
        "{}",
        import_report_summary(&report, &journal, args.download_images)
    );
    for failure in &report.failures {
        eprintln!("  ! {failure}");
    }
    Ok(())
}

fn import_report_summary(
    report: &journal_import::ImportReport,
    journal: &str,
    download_images: bool,
) -> String {
    let mut parts = vec![format!(
        "Imported {} {} into '{journal}'",
        report.imported,
        plural(report.imported, "entry", "entries"),
    )];
    if report.skipped_duplicate > 0 {
        parts.push(format!(
            "{} already imported (skipped)",
            report.skipped_duplicate
        ));
    }
    if report.images_stored > 0 {
        parts.push(format!(
            "{} {} stored",
            report.images_stored,
            plural(report.images_stored, "image", "images")
        ));
    }
    if report.attachments_skipped > 0 {
        parts.push(format!(
            "{} audio/video/pdf {} skipped (not yet supported)",
            report.attachments_skipped,
            plural(report.attachments_skipped, "attachment", "attachments")
        ));
    }
    if report.remote_images_skipped > 0 {
        if download_images {
            parts.push(format!(
                "{} offline {} replaced with [Offline Image]",
                report.remote_images_skipped,
                plural(report.remote_images_skipped, "image", "images")
            ));
        } else {
            parts.push(format!(
                "{} remote {} left as links (pass --download-images to fetch)",
                report.remote_images_skipped,
                plural(report.remote_images_skipped, "link", "links")
            ));
        }
    }
    if report.images_failed > 0 {
        parts.push(format!(
            "{} {} not stored",
            report.images_failed,
            plural(report.images_failed, "image", "images")
        ));
    }
    parts.join("; ")
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

fn validate_no_legacy_entry_args(cli: &Cli) -> AppResult<()> {
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
        if !MOOD_RANGE.contains(&score) {
            return Err(format!(
                "--mood must be between {} and {}, got {score}",
                MOOD_RANGE.start(),
                MOOD_RANGE.end()
            )
            .into());
        }
        Some(score)
    } else {
        None
    };
    let metadata = Metadata {
        tags,
        people,
        activities,
        feelings,
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

        Some(store.create_entry_with_body(journal, &body, &metadata)?)
    } else {
        let editor_cmd = config.editor.clone();
        store.create_entry_via_editor(journal, &metadata, |body| {
            editor::edit_body(&editor_cmd, body)
        })?
    };
    if let Some(path) = path {
        let report = store.process_entry_assets(&path, config.download_remote_images, false)?;
        if !report.is_noop() {
            eprintln!("{}", asset_report_message(&report));
        }
        println!("{}", path.display());
    }
    Ok(())
}

fn asset_report_message(report: &journal_storage::AssetReport) -> String {
    let mut parts = Vec::new();
    if report.stored > 0 {
        parts.push(format!(
            "{} {} stored",
            report.stored,
            plural(report.stored, "image", "images")
        ));
    }
    if report.removed > 0 {
        parts.push(format!("{} removed", report.removed));
    }
    if !report.failed.is_empty() {
        parts.push(format!(
            "{} {} not stored",
            report.failed.len(),
            plural(report.failed.len(), "image", "images")
        ));
    }
    parts.join("; ")
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
