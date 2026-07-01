use crate::AppResult;
use std::{fs, path::Path};

mod entries;
mod journals;
mod search;

pub use entries::{
    Entry, create_entry, entry_path, entry_template, move_entry_to_trash, open_editor, read_entry,
    scan_entries, set_updated_at_now,
};
pub use journals::{Journal, create_journal, list_journals, validate_journal_name};
pub use search::{SearchHit, SearchScopeFilter, search_entries};

pub fn ensure_workspace(root: &Path) -> AppResult<()> {
    fs::create_dir_all(root)?;
    Ok(())
}
