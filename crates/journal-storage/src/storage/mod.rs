use crate::AppResult;
use std::{fs, path::Path};

mod dates;
mod entry;
mod journals;

pub use dates::{entry_group_date, entry_timestamp_label, parse_entry_timestamp};
pub(crate) use entry::entry_date_from_path;
pub use entry::Entry;
pub use entry::entry_id;
pub(crate) use entry::{
    collect_entry_paths, create_encrypted_entry_with_body_and_metadata,
    create_entry_with_body_and_metadata, delete_empty_entry, delete_journal, edit_entry_body,
    is_encrypted_entry_file, is_plain_entry_file, move_entry_to_trash,
    read_entries, read_entry_content_with_identity, read_entry_with_identity,
    scan_entries_with_identity, write_encrypted_entry_content,
};
pub use journals::{Journal, create_journal, list_journals, validate_journal_name};

pub fn ensure_store(root: &Path) -> AppResult<()> {
    fs::create_dir_all(root)?;
    Ok(())
}
