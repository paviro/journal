use crate::AppResult;
use std::{fs, path::Path};

mod dates;
mod entries;
mod journals;
mod search;

pub(crate) use dates::{entry_group_date, entry_timestamp_label, parse_entry_timestamp};
pub use entries::{
    Entry, EntryEncryptionState, create_encrypted_entry, create_encrypted_entry_with_body,
    create_entry, create_entry_with_body, edit_encrypted_entry, entry_path, entry_template,
    has_encrypted_entries, is_encrypted_entry_file, is_entry_file, is_plain_entry_file,
    move_entry_to_trash, open_editor, read_entry, read_entry_content,
    read_entry_content_with_identity, read_entry_with_identity, scan_entries,
    scan_entries_with_identity, set_updated_at_now,
};
pub use journals::{Journal, create_journal, list_journals, validate_journal_name};
pub use search::{SearchHit, SearchScopeFilter, search_entries, search_entries_with_identity};

pub fn ensure_workspace(root: &Path) -> AppResult<()> {
    fs::create_dir_all(root)?;
    Ok(())
}
