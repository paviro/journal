use crate::AppResult;
use std::{fs, path::Path};

mod dates;
mod entry;
mod journal_metadata;
mod journals;

pub use dates::{entry_timestamp_label, parse_entry_timestamp};
pub use entry::entry_id;
pub use entry::is_entry_file;
pub use entry::sole_stored_image;
pub use entry::{
    AssetFailure, AssetReport, EditOutcome, EntryAssetOptions, EntryCreateOutcome, EntryDraft,
    EntryEdit, EntryEditOutcome,
};
pub(crate) use entry::{
    EntryCodec, collect_discovered_entries_with_progress, collect_entry_paths, create_entry,
    create_entry_copy, delete_empty_entry, delete_journal, is_encrypted_entry_file,
    is_plain_entry_file, move_entry_to_trash, random_id, read_entries, read_entries_with_progress,
    read_entry, read_entry_content, save_entry_edit, save_entry_edit_if_revision,
    scan_import_sources,
};
pub use entry::{resolve_entry_asset_path, stored_asset_reference, stored_asset_reference_for};
pub use journal_metadata::JournalTheme;
pub(crate) use journal_metadata::set_theme as set_journal_theme;
pub use journals::{ARCHIVED_SUFFIX, Journal, is_archived_name, journal_display_name};
pub(crate) use journals::{
    create_journal, discover_journals, initialize_journals, list_journals, set_journal_archived,
    validate_journal_name,
};

pub(crate) fn ensure_store(root: &Path) -> AppResult<()> {
    fs::create_dir_all(root)?;
    Ok(())
}
