use crate::{AppResult, config, editor, encryption_cli, prompts, tui};
use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use journal_core::feelings;
use journal_core::{MOOD_RANGE, Metadata};
use journal_storage::{JournalStore, SecretString};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

#[derive(Debug, Parser)]
#[command(name = "journal")]
#[command(about = "Markdown terminal journal")]
struct Cli {
    /// Config directory holding config.toml and this device's encryption key;
    /// defaults to $XDG_CONFIG_HOME/journal, else ~/.config/journal (macOS:
    /// ~/Library/Application Support/de.paviro.journal). Global, so it works
    /// before or after a subcommand.
    #[arg(long, value_name = "DIR", global = true)]
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
    /// Import entries from another journaling app
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },
    /// Manage journal encryption: enable/disable and the device keystore
    #[command(alias = "enc")]
    Encryption {
        #[command(subcommand)]
        command: EncryptionCommand,
    },
    /// Show data-source attributions and third-party dependency licenses.
    /// Pass a dependency name to print its full license text.
    Licenses {
        /// Show the full license text for a specific dependency
        #[arg(value_name = "DEPENDENCY")]
        dependency: Option<String>,
    },
    /// Mount the journal as a decrypted, writable filesystem
    #[cfg(feature = "fuse")]
    Mount {
        /// Directory to mount at (created if missing). Omit to use a temporary
        /// directory — on macOS the journal still appears as a drive in Finder.
        #[arg(value_name = "MOUNTPOINT")]
        mountpoint: Option<PathBuf>,
    },
    /// Fill a journal with backdated fake entries [debug builds only]
    #[cfg(debug_assertions)]
    Sample(SampleArgs),
}

/// Arguments for the debug-only `sample` command.
#[cfg(debug_assertions)]
#[derive(Debug, Args)]
struct SampleArgs {
    /// Journal to fill; created if it doesn't exist
    #[arg(value_name = "NAME", default_value = "Sample")]
    journal: String,

    /// Number of entries to generate
    #[arg(long, default_value_t = 750)]
    count: usize,

    /// Spread creation dates across the last N days
    #[arg(long, default_value_t = 1095)]
    days: i64,

    /// Seed the generator for a reproducible dataset
    #[arg(long)]
    seed: Option<u64>,
}

#[derive(Debug, Subcommand)]
enum EncryptionCommand {
    /// Turn on encryption for this device (creating its key if needed) and encrypt every plaintext entry
    Enable(NewIdentityArgs),
    /// Decrypt every encrypted entry, turning encryption off
    Disable(ConfirmArgs),
    /// Manage the devices that can read this encrypted journal
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DeviceCommand {
    /// Request access for this device to an already-encrypted journal (approve it from an existing device)
    Enroll(NewIdentityArgs),
    /// List the devices that can read this journal, plus pending requests
    List,
    /// Revoke a device and re-encrypt all entries to exclude it
    Revoke {
        #[arg(value_name = "NAME")]
        name: String,
        #[command(flatten)]
        confirm: ConfirmArgs,
    },
    /// Rename a device's label (no re-encryption)
    Rename {
        #[arg(value_name = "OLD")]
        old: String,
        #[arg(value_name = "NEW")]
        new: String,
    },
    /// Approve pending device-access requests (add + re-encrypt)
    Approve(RequestSelectionArgs),
    /// Reject pending device-access requests without granting access
    Reject(RequestSelectionArgs),
    /// Add, remove, or change this device's key passphrase
    Passphrase(PassphraseArgs),
    /// Replace this device's key and re-encrypt, retiring the old key
    Rotate,
}

#[derive(Debug, Args)]
struct PassphraseArgs {
    /// Remove the passphrase, storing the key unprotected
    #[arg(long)]
    remove: bool,
    #[command(flatten)]
    confirm: ConfirmArgs,
}

/// Shared `--yes`/`-y` flag that skips the confirmation prompt on a destructive
/// operation, for scripting and non-interactive use.
#[derive(Debug, Args)]
struct ConfirmArgs {
    /// Skip the confirmation prompt
    #[arg(long, short = 'y')]
    yes: bool,
}

/// Options for creating a new device identity, shared by `encryption enable`
/// (first key on this device) and `device enroll` (joining an existing store).
#[derive(Debug, Args)]
struct NewIdentityArgs {
    /// Name for this device when creating a new identity (prompted if omitted)
    #[arg(long, value_name = "NAME")]
    name: Option<String>,

    /// Create the key without a passphrase; it opens automatically. Omit to be
    /// asked interactively whether to protect the key with a passphrase.
    #[arg(long)]
    no_passphrase: bool,
}

/// Which pending join requests a command acts on. Shared by `approve` and
/// `reject`: name/id selects one, `--all` selects every queued request.
#[derive(Debug, Args)]
struct RequestSelectionArgs {
    /// Act only on the request whose name or id matches
    #[arg(value_name = "NAME_OR_ID")]
    which: Option<String>,

    /// Act on every pending request
    #[arg(long)]
    all: bool,
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
        bail!("piped entry text requires `journal log`; run `journal log` with piped stdin");
    }

    let config::Startup {
        config_path,
        config,
        store,
    } = config::load_or_setup_with_path(cli.config.as_deref())?;
    tui::run(config_path, config, *store)
}

fn handle_command(cli: &Cli, command: &CliCommand, stdin_is_pipe: bool) -> AppResult<()> {
    validate_no_legacy_entry_args(cli)?;
    match command {
        CliCommand::Log(args) => create_entry_from_log_command(cli, args, stdin_is_pipe),
        CliCommand::Use { name } => set_default_journal(cli, name),
        CliCommand::Import { source } => match source {
            ImportSource::Dayone(args) => import_dayone_command(cli, args),
        },
        CliCommand::Encryption { command } => handle_encryption_command(cli, command),
        CliCommand::Licenses { dependency } => crate::licenses::run(dependency.clone()),
        #[cfg(feature = "fuse")]
        CliCommand::Mount { mountpoint } => mount_command(cli, mountpoint.as_deref()),
        #[cfg(debug_assertions)]
        CliCommand::Sample(args) => generate_sample_data(cli, args),
    }
}

/// Populate a journal with fake entries for development. Debug builds only —
/// the whole command is compiled out of release binaries.
#[cfg(debug_assertions)]
fn generate_sample_data(cli: &Cli, args: &SampleArgs) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let store = JournalStore::for_config(&config_path, &config.journal.path)?;
    store.ensure()?;

    let created = journal_seed::generate(
        &store,
        &journal_seed::GenConfig {
            journal: args.journal.clone(),
            count: args.count,
            days: args.days,
            seed: args.seed,
        },
    )?;
    println!(
        "Generated {created} sample {} in journal \"{}\".",
        if created == 1 { "entry" } else { "entries" },
        args.journal
    );
    Ok(())
}

fn handle_encryption_command(cli: &Cli, command: &EncryptionCommand) -> AppResult<()> {
    match command {
        EncryptionCommand::Enable(args) => {
            let (config_path, config) = config::load_existing(cli.config.as_deref())?;
            encryption_cli::encrypt_store(
                &config_path,
                &config,
                args.name.as_deref(),
                args.no_passphrase,
            )
        }
        EncryptionCommand::Disable(args) => {
            let (config_path, config) = config::load_existing(cli.config.as_deref())?;
            if !prompts::confirm(
                "Decrypt every entry and turn encryption off for this journal?",
                args.yes,
            )? {
                println!("Aborted.");
                return Ok(());
            }
            encryption_cli::decrypt_store(&config_path, &config)
        }
        EncryptionCommand::Device { command } => handle_device_command(cli, command),
    }
}

fn handle_device_command(cli: &Cli, command: &DeviceCommand) -> AppResult<()> {
    match command {
        DeviceCommand::Enroll(args) => device_enroll_command(cli, args),
        DeviceCommand::List => device_list_command(cli),
        DeviceCommand::Revoke { name, confirm } => device_revoke_command(cli, name, confirm.yes),
        DeviceCommand::Rename { old, new } => device_rename_command(cli, old, new),
        DeviceCommand::Approve(args) => device_approve_command(cli, args),
        DeviceCommand::Reject(args) => device_reject_command(cli, args),
        DeviceCommand::Passphrase(args) => device_passphrase_command(cli, args),
        DeviceCommand::Rotate => device_rotate_command(cli),
    }
}

/// Open the store and unlock this device's identity, prompting for a passphrase
/// only when the identity is passphrase-protected. Returns the passphrase too
/// (for rotation, which re-wraps the new key with it). Used by the device
/// operations that must decrypt in order to re-encrypt.
fn open_unlocked_store_with_passphrase(
    cli: &Cli,
) -> AppResult<(JournalStore, Option<SecretString>)> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let mut store = JournalStore::for_config(&config_path, &config.journal.path)?;
    store.ensure()?;
    if !store.unlock_available() {
        bail!(
            "no encryption identity on this device; run `{}` first",
            crate::ENROLL_CMD
        );
    }
    let passphrase = if store.identity_needs_passphrase()? {
        Some(prompts::prompt_unlock_passphrase()?)
    } else {
        None
    };
    store.unlock(passphrase.as_ref())?;
    Ok((store, passphrase))
}

fn open_unlocked_store(cli: &Cli) -> AppResult<JournalStore> {
    Ok(open_unlocked_store_with_passphrase(cli)?.0)
}

/// Mount the whole journal store as a decrypted filesystem. Journals appear as
/// top-level folders; entries and their assets are decrypted on read and
/// re-encrypted on write. Only encrypted journals can be mounted — for a
/// plaintext journal a mount would add nothing over the files already on disk.
/// The identity is unlocked first, prompting only when the key is passphrase-
/// protected. Blocks until unmounted.
///
/// With no `mountpoint` a temporary directory is created and used (on macOS the
/// journal still shows up as a drive in Finder); an explicit path is created if
/// it doesn't exist. Either way, a directory we created is removed after unmount.
#[cfg(feature = "fuse")]
fn mount_command(cli: &Cli, mountpoint: Option<&Path>) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let mut store = JournalStore::for_config(&config_path, &config.journal.path)?;
    store.ensure()?;

    if !store.encryption_enabled() {
        bail!(
            "`journal mount` is only for encrypted journals; this journal is not encrypted. \
             Enable encryption with `journal encryption enable`, or open the files directly."
        );
    }
    if !store.unlock_available() {
        bail!(
            "this journal is encrypted but this device has no key; run `{}` first",
            crate::ENROLL_CMD
        );
    }

    // Resolve the mount point. An explicit path is created if missing; with none,
    // fall back to a fresh temp directory. `created` tracks whether we made the
    // directory so we can remove it again on unmount and leave nothing behind.
    let (mount_path, created): (PathBuf, bool) = match mountpoint {
        Some(path) if path.exists() => {
            if !path.is_dir() {
                bail!("mount point {} is not a directory", path.display());
            }
            (path.to_path_buf(), false)
        }
        Some(path) => {
            std::fs::create_dir_all(path)
                .with_context(|| format!("creating mount point {}", path.display()))?;
            (path.to_path_buf(), true)
        }
        None => {
            let path = std::env::temp_dir().join(format!("journal-mount-{}", std::process::id()));
            std::fs::create_dir_all(&path)
                .with_context(|| format!("creating mount point {}", path.display()))?;
            (path, true)
        }
    };

    let passphrase = if store.identity_needs_passphrase()? {
        Some(prompts::prompt_unlock_passphrase()?)
    } else {
        None
    };
    store.unlock(passphrase.as_ref())?;

    println!(
        "Mounting journal at {}. Unmount with `umount {}` (macOS: `diskutil unmount`) or Ctrl-C.",
        mount_path.display(),
        mount_path.display()
    );
    journal_fuse::mount(store, &mount_path)?;
    println!("Unmounted {}.", mount_path.display());

    // Best-effort cleanup after unmount, only for a directory we created (and
    // only while empty — the mount left it as we found it). Ctrl-C kills the
    // process before this runs; the empty directory is harmless if it lingers.
    if created {
        let _ = std::fs::remove_dir(&mount_path);
    }
    Ok(())
}

fn device_passphrase_command(cli: &Cli, args: &PassphraseArgs) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let store = JournalStore::for_config(&config_path, &config.journal.path)?;
    store.ensure()?;
    let Some(info) = store.this_device()? else {
        bail!(
            "no encryption identity on this device; run `{}` first",
            crate::ENROLL_CMD
        );
    };

    if args.remove
        && !prompts::confirm(
            "Remove the passphrase, storing this device's key unprotected?",
            args.confirm.yes,
        )?
    {
        println!("Aborted.");
        return Ok(());
    }

    let current = if info.passphrase_protected {
        Some(prompts::prompt_unlock_passphrase()?)
    } else {
        None
    };
    let new = if args.remove {
        None
    } else {
        Some(prompts::prompt_new_passphrase()?)
    };
    store.set_passphrase(current.as_ref(), new.as_ref())?;

    if new.is_some() {
        println!("Updated this device's key passphrase.");
    } else {
        println!(
            "Removed the passphrase; the key now opens automatically. Keep this device secure."
        );
    }
    Ok(())
}

fn device_rotate_command(cli: &Cli) -> AppResult<()> {
    let (mut store, passphrase) = open_unlocked_store_with_passphrase(cli)?;
    let summary = store.rotate_identity(passphrase.as_ref(), encryption_cli::cli_progress())?;
    println!(
        "Rotated this device's key and re-encrypted {} file(s).",
        summary.migrated_files
    );
    println!("The previous key can no longer read this journal.");
    Ok(())
}

fn device_enroll_command(cli: &Cli, args: &NewIdentityArgs) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let store = JournalStore::for_config(&config_path, &config.journal.path)?;
    store.ensure()?;
    if !store.encryption_enabled() {
        bail!(
            "this journal is not encrypted yet; run `journal encryption enable` to turn it on for this device"
        );
    }
    if store.unlock_available() {
        let name = store
            .this_device()?
            .map(|device| device.name)
            .unwrap_or_default();
        bail!(
            "this device already has an identity ('{name}') at {}.\n\
             If you're waiting for approval, run `journal encryption device list` to see the \
             request, or approve it from a device that can already read this journal.\n\
             To start over, delete that identity file and re-run enroll.",
            store.paths().keys.identity_file.display()
        );
    }

    let (name, passphrase) =
        prompts::resolve_new_identity_options(args.name.as_deref(), args.no_passphrase)?;

    // Joining a store that already exists (its recipients synced here): drop a
    // request for a device that can decrypt to approve.
    let recipient = store.request_access(&name, passphrase.as_ref())?;
    println!("Requested access as '{name}'. Your public recipient (safe to share):");
    println!("  {}", recipient.enc_key);
    println!(
        "Fingerprint (read this out to confirm it on the approving device):\n  {}",
        recipient.fingerprint()
    );
    println!(
        "On a device that can already read this journal, approve it — this request\nappears in `journal encryption device list` and a modal at launch — then run there:"
    );
    println!("  {} {name}", crate::APPROVE_CMD);
    println!(
        "Identity file: {}. Back it up; without it encrypted entries cannot be decrypted.",
        store.paths().keys.identity_file.display()
    );
    if passphrase.is_none() {
        println!("This key has no passphrase — keep this device and its backups secure.");
    }
    Ok(())
}

fn device_list_command(cli: &Cli) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let store = JournalStore::for_config(&config_path, &config.journal.path)?;
    store.ensure()?;

    let recipients = store.recipients()?;
    if recipients.is_empty() {
        println!("This journal is not encrypted.");
        return Ok(());
    }

    let this_device = store.this_device()?;
    println!("Recipients:");
    for recipient in &recipients {
        let marker = if this_device
            .as_ref()
            .is_some_and(|device| device.name == recipient.name)
        {
            "  (this device)"
        } else {
            ""
        };
        println!("  {}  {}{marker}", recipient.name, recipient.enc_key);
        println!("      fingerprint: {}", recipient.fingerprint());
    }

    let pending = store.pending_requests()?;
    if !pending.is_empty() {
        println!("\nPending approval (run `{}`):", crate::APPROVE_CMD);
        println!("Confirm each fingerprint out-of-band before approving.");
        for request in &pending {
            println!(
                "  {}  {}  [{}]",
                request.recipient.name, request.recipient.enc_key, request.id
            );
            println!("      fingerprint: {}", request.recipient.fingerprint());
        }
    }
    Ok(())
}

fn device_revoke_command(cli: &Cli, name: &str, skip_confirm: bool) -> AppResult<()> {
    if !prompts::confirm(
        &format!("Revoke '{name}' and re-encrypt all entries to exclude it?"),
        skip_confirm,
    )? {
        println!("Aborted.");
        return Ok(());
    }
    let store = open_unlocked_store(cli)?;
    let summary = store.revoke_recipient(name, encryption_cli::cli_progress())?;
    println!(
        "Revoked '{name}' and re-encrypted {} file(s).",
        summary.migrated_files
    );
    println!("Revocation is forward-only: entries that device already synced stay readable to it.");
    Ok(())
}

fn device_rename_command(cli: &Cli, old: &str, new: &str) -> AppResult<()> {
    let store = open_unlocked_store(cli)?;
    store.rename_recipient(old, new)?;
    println!("Renamed '{old}' to '{new}'.");
    Ok(())
}

/// The pending requests an `approve`/`reject` invocation targets: `--all` picks
/// every queued request, otherwise `which` matches a request by id or device
/// name. `action` names the operation in the "how to select" error. Errors if
/// nothing was selected or matched; the empty-queue case is handled by callers.
fn select_requests(
    pending: Vec<journal_storage::PendingRequest>,
    args: &RequestSelectionArgs,
    action: &str,
) -> AppResult<Vec<journal_storage::PendingRequest>> {
    let selected: Vec<_> = if args.all {
        pending
    } else if let Some(which) = &args.which {
        pending
            .into_iter()
            .filter(|request| &request.id == which || &request.recipient.name == which)
            .collect()
    } else {
        bail!("specify a device name or id to {action}, or pass --all");
    };
    if selected.is_empty() {
        bail!("no pending request matched");
    }
    Ok(selected)
}

fn device_approve_command(cli: &Cli, args: &RequestSelectionArgs) -> AppResult<()> {
    let store = open_unlocked_store(cli)?;
    let pending = store.pending_requests()?;
    if pending.is_empty() {
        println!("No pending requests.");
        return Ok(());
    }

    for request in select_requests(pending, args, "approve")? {
        let summary = store.approve_pending(&request, encryption_cli::cli_progress())?;
        println!(
            "Approved '{}' and re-encrypted {} file(s).",
            request.recipient.name, summary.migrated_files
        );
    }
    Ok(())
}

fn device_reject_command(cli: &Cli, args: &RequestSelectionArgs) -> AppResult<()> {
    // Rejecting only deletes the request file, so no unlock/re-encryption needed.
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let store = JournalStore::for_config(&config_path, &config.journal.path)?;
    store.ensure()?;
    let pending = store.pending_requests()?;
    if pending.is_empty() {
        println!("No pending requests.");
        return Ok(());
    }

    for request in select_requests(pending, args, "reject")? {
        store.deny_pending(&request)?;
        println!("Rejected '{}'.", request.recipient.name);
    }
    Ok(())
}

fn import_dayone_command(cli: &Cli, args: &DayoneArgs) -> AppResult<()> {
    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let journal = args
        .journal
        .as_deref()
        .or(config.journal.default.as_deref())
        .context("no journal specified; pass --journal or set one with `journal use <name>`")?;
    // Validate the name only — the importer creates the journal if it's missing.
    let journal = JournalStore::validate_journal_name(journal)?;

    let store = JournalStore::for_config(&config_path, &config.journal.path)?;
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
        bail!("--journal belongs to `journal log`");
    }
    if !cli.tag.is_empty() {
        bail!("--tag belongs to `journal log`");
    }
    if !cli.person.is_empty() {
        bail!("--person belongs to `journal log`");
    }
    if !cli.activity.is_empty() {
        bail!("--activity belongs to `journal log`");
    }
    if !cli.feeling.is_empty() {
        bail!("--feeling belongs to `journal log`");
    }
    if cli.mood.is_some() {
        bail!("--mood belongs to `journal log`");
    }
    Ok(())
}

fn set_default_journal(cli: &Cli, journal: &str) -> AppResult<()> {
    let (path, mut config) = config::load_existing(cli.config.as_deref())?;
    validate_existing_journal(&config.journal.path, journal)?;
    config.journal.default = Some(journal.to_string());
    config::save_config(&path, &config)?;
    println!("Default journal set to {journal}");
    Ok(())
}

fn create_entry_from_log_command(cli: &Cli, args: &LogArgs, stdin_is_pipe: bool) -> AppResult<()> {
    let body_from_args = !args.body.is_empty();
    if body_from_args && stdin_is_pipe {
        bail!("entry text cannot be combined with piped stdin");
    }

    let (config_path, config) = config::load_existing(cli.config.as_deref())?;
    let journal = args
        .journal
        .as_deref()
        .or(config.journal.default.as_deref())
        .context("no journal specified; pass --journal or set one with `journal use <name>`")?;
    validate_existing_journal(&config.journal.path, journal)?;
    let tags = comma_separated_values(&args.tag);
    let people = comma_separated_values(&args.person);
    let activities = comma_separated_values(&args.activity);
    let feelings = feelings::validate_feelings(
        args.feeling
            .iter()
            .flat_map(|f| f.split(','))
            .map(str::trim)
            .filter(|f| !f.is_empty()),
    )
    .map_err(anyhow::Error::msg)?;
    let mood = if let Some(score) = args.mood {
        if !MOOD_RANGE.contains(&score) {
            bail!(
                "--mood must be between {} and {}, got {score}",
                MOOD_RANGE.start(),
                MOOD_RANGE.end()
            );
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
        starred: false,
        location: None,
    };

    let store = JournalStore::for_config(&config_path, &config.journal.path)?;
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
        let editor_cmd = config.editor.command.clone();
        store.create_entry_via_editor(journal, &metadata, |body| {
            editor::edit_body(&editor_cmd, body)
        })?
    };
    if let Some(path) = path {
        let report =
            store.process_entry_assets(&path, config.attachments.download_remote_images, false)?;
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
    use std::io::IsTerminal;
    !io::stdin().is_terminal()
}

fn validate_existing_journal(root: &Path, journal: &str) -> AppResult<()> {
    let journal = JournalStore::validate_journal_name(journal)?;
    let path = root.join(&journal);
    if !path.is_dir() {
        bail!("journal '{journal}' does not exist");
    }
    Ok(())
}
