use crate::AppResult;
use std::{fs, path::Path};

mod dates;
mod entry;
mod journals;

pub use dates::{entry_group_date, entry_timestamp_label, parse_entry_timestamp};
pub use entry::Entry;
pub(crate) use entry::entry_date_from_path;
pub use entry::entry_id;
pub use entry::is_entry_file;
pub use entry::sole_stored_image;
pub use entry::stored_image_reference;
pub use entry::{AssetFailure, AssetReport};
pub(crate) use entry::{
    EntryCodec, collect_entry_paths, create_entry, create_imported_entry, delete_empty_entry,
    delete_journal, edit_entry_body, ingest_and_cleanup_opts, is_encrypted_entry_file,
    is_plain_entry_file, move_entry_to_trash, read_entries, read_entry, read_entry_content,
    resolve_entry_asset_path, scan_entries,
};
pub use journals::{Journal, create_journal, list_journals, validate_journal_name};

pub fn ensure_store(root: &Path) -> AppResult<()> {
    fs::create_dir_all(root)?;
    Ok(())
}
