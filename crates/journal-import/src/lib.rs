//! Import external journals into the store.
//!
//! Currently supports [Day One](https://dayoneapp.com/) JSON exports via
//! [`import_dayone`]. Each importer maps an external format onto the store's
//! entry model, records provenance (`import_id`) so re-runs skip already-imported
//! entries, and preserves original timestamps.

mod dayone;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use journal_storage::{AppResult, AssetFailure, JournalStore, Metadata, parse_entry_timestamp};

use dayone::model::DayOneExport;
use dayone::moments::{MediaIndex, rewrite_moments};
use dayone::richtext;
use dayone::text::{
    merge_code_fences, normalize_whitespace, recover_html_embeds, unescape_markdown,
};

/// Summary of a Day One import, printed to the user.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Entries created.
    pub imported: usize,
    /// Entries skipped because their `import_id` was already present.
    pub skipped_duplicate: usize,
    /// Photos copied into entry asset folders.
    pub images_stored: usize,
    /// Photos that could not be ingested (missing file, decode failure, …).
    pub images_failed: usize,
    /// Remote `http(s)` images that were not fetched. When downloading was on,
    /// these were unreachable and are replaced in the body with `[Offline
    /// Image]`; when off, they are left as links to fetch later. Not failures.
    pub remote_images_skipped: usize,
    /// Non-image attachments (audio/video/pdf) referenced but not imported.
    pub attachments_skipped: usize,
    /// Human-readable per-entry problems that did not abort the import.
    pub failures: Vec<String>,
}

/// Import every entry from a Day One JSON export at `json_path` into `journal`,
/// creating the journal if it does not exist. Media folders (e.g. `photos/`) are
/// resolved relative to the JSON file. Entries whose Day One UUID was already
/// imported are skipped.
///
/// `download_remote` gates fetching `http(s)` image links found in entry bodies
/// (Day One entries can embed remote images, distinct from local `photos`);
/// pass the store's configured preference, mirroring `journal log`.
pub fn import_dayone(
    store: &JournalStore,
    journal: &str,
    json_path: &Path,
    download_remote: bool,
) -> AppResult<ImportReport> {
    let raw = fs::read_to_string(json_path)
        .map_err(|error| format!("could not read {}: {error}", json_path.display()))?;
    let export: DayOneExport = serde_json::from_str(&raw)
        .map_err(|error| format!("could not parse Day One export: {error}"))?;
    let media_root = json_path.parent().unwrap_or_else(|| Path::new("."));

    if !store.list_journals()?.iter().any(|j| j.name == journal) {
        store.create_journal(journal)?;
    }

    let mut seen: HashSet<String> = store
        .scan_entries()?
        .into_iter()
        .filter_map(|entry| entry.import_id)
        .collect();

    let mut report = ImportReport::default();

    for entry in &export.entries {
        let import_id = format!("dayone:{}", entry.uuid);
        if seen.contains(&import_id) {
            report.skipped_duplicate += 1;
            continue;
        }

        let Some(created_at) = entry
            .creation_date
            .as_deref()
            .and_then(parse_entry_timestamp)
        else {
            report
                .failures
                .push(format!("{}: missing or invalid creationDate", entry.uuid));
            continue;
        };
        let edited_at = entry
            .modified_date
            .as_deref()
            .and_then(parse_entry_timestamp)
            .unwrap_or(created_at);

        let media = MediaIndex::build(entry, media_root);
        // Prefer Day One's structured `richText` (clean, faithful) when present;
        // otherwise clean up its lossy `text`. `richtext::render` yields `None`
        // when `richText` is absent *or* parses to empty, so either way we fall
        // through to the `text` path. Both leave images as `dayone-moment://`
        // references for `rewrite_moments` below.
        let body = entry
            .rich_text
            .as_deref()
            .and_then(richtext::render)
            .unwrap_or_else(|| {
                let text = entry.text.as_deref().unwrap_or_default();
                let cleaned = normalize_whitespace(&unescape_markdown(text));
                recover_html_embeds(&merge_code_fences(&cleaned))
            });
        let rewrite = rewrite_moments(&body, &media);

        let metadata = Metadata {
            tags: entry.tags.clone(),
            ..Metadata::default()
        };

        let path = store.create_imported_entry(
            journal,
            &rewrite.body,
            &metadata,
            created_at,
            edited_at,
            &import_id,
        )?;
        // Replace un-fetchable images with a placeholder only when we actually
        // tried to download — otherwise remote links are kept so they can be
        // fetched by a later `--download-images` run.
        let assets = store.process_entry_assets(&path, download_remote, download_remote)?;

        report.images_stored += assets.stored;
        for failure in assets.failed {
            match failure {
                // A remote link we chose not to (or couldn't) fetch — download
                // off, or the host is gone — is left in the body as a link, not
                // a failure.
                AssetFailure::RemoteUnavailable { .. } => report.remote_images_skipped += 1,
                AssetFailure::Ingest { source, error } => {
                    report.images_failed += 1;
                    report
                        .failures
                        .push(format!("{}: {source}: {error}", entry.uuid));
                }
            }
        }
        for id in &rewrite.unresolved {
            report
                .failures
                .push(format!("{}: unresolved photo moment {id}", entry.uuid));
        }
        report.attachments_skipped += rewrite.skipped_attachments();
        report.imported += 1;
        seen.insert(import_id);
    }

    Ok(report)
}
