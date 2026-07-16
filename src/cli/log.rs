use std::{
    io::{self, IsTerminal, Read},
    path::Path,
};

use anyhow::{Context, bail};
use chrono::Local;
use notema_context::{EnvironmentProvider, fetch_environment, resolve_zone, rezone};
use notema_domain::{Location, MOOD_RANGE, Metadata, validate_feelings};
use notema_storage::JournalStore;

use crate::{AppResult, startup, tui};

use super::location::{self, ResolvedLocation};
use super::{Cli, LogArgs, plural};

pub(super) fn run(cli: &Cli, args: &LogArgs, stdin_is_pipe: bool) -> AppResult<()> {
    let body_from_args = !args.body.is_empty();
    if body_from_args && stdin_is_pipe {
        bail!("entry text cannot be combined with piped stdin");
    }

    let startup::Startup {
        config_path,
        config,
        store,
        ..
    } = startup::load_existing(cli.config.as_deref())?;
    let journal = args
        .journal
        .as_deref()
        .or(config.journal.default.as_deref())
        .context("no journal specified; pass --journal or set one with `notema use <name>`")?;
    validate_existing_journal(&config.journal.path, journal)?;

    // No inline text: compose interactively in the fullscreen built-in editor. Its
    // own on-screen shortcuts set tags/people/mood and location (Ctrl+L, which
    // also fetches environment), so the metadata and --location flags apply only
    // to a one-shot logged entry — reject them here rather than silently drop them.
    if !body_from_args && !stdin_is_pipe {
        if let Some(flag) = first_interactive_only_flag(args) {
            bail!(
                "{flag} applies only to a one-shot entry with inline text; open the editor and set it with the on-screen shortcuts"
            );
        }
        let journal = journal.to_string();
        return tui::run_compose(config_path, config, store, journal, Metadata::default());
    }

    let tags = comma_separated_values(&args.tag);
    let people = comma_separated_values(&args.person);
    let activities = comma_separated_values(&args.activity);
    let feelings = validate_feelings(
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
    // A numbered picker for an ambiguous address only works when it can be shown
    // and answered: stdout is a terminal and stdin isn't the piped entry body.
    let interactive = io::stdout().is_terminal() && !stdin_is_pipe;
    let (location, osm_timezone) = match location::resolve(args.location.clone(), interactive)? {
        Some(ResolvedLocation {
            location,
            osm_timezone,
        }) => (Some(location), osm_timezone),
        None => (None, None),
    };

    let metadata = Metadata {
        tags,
        people,
        activities,
        feelings,
        mood,
        starred: false,
        location,
    };

    let body = if body_from_args {
        args.body.join(" ")
    } else {
        let mut body = String::new();
        io::stdin().read_to_string(&mut body)?;
        body
    };

    // A located entry adopts its place's timezone (config-gated) so its timestamp
    // and date-folder match where it was written, and captures the ambient
    // weather/air/celestial there — the same enrichment the TUI performs.
    let mut created_at = Local::now().fixed_offset();
    let mut timezone = None;
    let mut environment = None;
    if let Some(coordinates) = metadata.location.as_ref().and_then(Location::coordinates) {
        if config.location.use_location_timezone
            && let Some(zone) = resolve_zone(coordinates, osm_timezone.as_deref())
        {
            created_at = rezone(created_at, zone);
            timezone = Some(zone.name().to_string());
        }
        let report = fetch_environment(coordinates, created_at);
        for warning in &report.warnings {
            eprintln!(
                "note: {} unavailable ({})",
                environment_provider_label(warning.provider),
                warning.message
            );
        }
        environment = Some(report);
    }

    let mut draft = notema_storage::EntryDraft::new(journal, &body, &metadata);
    if metadata.location.is_some() {
        draft.created_at = Some(created_at);
        draft.timezone = timezone.as_deref();
        if let Some(report) = &environment {
            draft.celestial = Some(&report.celestial);
            draft.weather = report.weather.as_ref();
            draft.air_quality = report.air_quality.as_ref();
        }
    }
    let created = store.create_entry(
        draft,
        notema_storage::EntryAssetOptions {
            download_remote: config.attachments.download_remote_images,
            replace_offline: false,
        },
    )?;
    if !created.assets.is_noop() {
        eprintln!("{}", asset_report_message(&created.assets));
    }
    println!("{}", created.path.display());
    Ok(())
}

fn asset_report_message(report: &notema_storage::AssetReport) -> String {
    let mut parts = Vec::new();
    let images_stored = report.images_stored();
    if images_stored > 0 {
        parts.push(format!(
            "{} {} stored",
            images_stored,
            plural(images_stored, "image", "images")
        ));
    }
    if report.attachments_stored > 0 {
        parts.push(format!(
            "{} {} stored",
            report.attachments_stored,
            plural(report.attachments_stored, "attachment", "attachments")
        ));
    }
    if report.removed > 0 {
        parts.push(format!("{} removed", report.removed));
    }
    let images_not_stored = report.images_not_stored();
    if images_not_stored > 0 {
        parts.push(format!(
            "{} {} not stored",
            images_not_stored,
            plural(images_not_stored, "image", "images")
        ));
    }
    let attachments_not_stored = report.attachments_not_stored();
    if attachments_not_stored > 0 {
        parts.push(format!(
            "{} {} not stored",
            attachments_not_stored,
            plural(attachments_not_stored, "attachment", "attachments")
        ));
    }
    parts.join("; ")
}

/// The first metadata/location flag that was supplied, if any. These enrich a
/// one-shot logged entry; when the command instead opens the fullscreen editor
/// (no inline text), they have no effect and are reported rather than ignored.
fn first_interactive_only_flag(args: &LogArgs) -> Option<&'static str> {
    if !args.tag.is_empty() {
        Some("--tag")
    } else if !args.person.is_empty() {
        Some("--person")
    } else if !args.activity.is_empty() {
        Some("--activity")
    } else if !args.feeling.is_empty() {
        Some("--feeling")
    } else if args.mood.is_some() {
        Some("--mood")
    } else if args.location.is_some() {
        Some("--location")
    } else {
        None
    }
}

fn environment_provider_label(provider: EnvironmentProvider) -> &'static str {
    match provider {
        EnvironmentProvider::Weather => "weather",
        EnvironmentProvider::AirQuality => "air quality",
    }
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

pub(super) fn validate_existing_journal(root: &Path, journal: &str) -> AppResult<()> {
    let journal = JournalStore::validate_journal_name(journal)?;
    let path = root.join(&journal);
    if !path.is_dir() {
        bail!(
            "journal '{journal}' does not exist; create it or pick another with `notema use <name>`"
        );
    }
    Ok(())
}
