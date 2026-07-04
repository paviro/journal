use std::path::PathBuf;

mod create;
mod edit;
mod paths;
mod read;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub journal: String,
    pub path: PathBuf,
    pub encryption_state: EntryEncryptionState,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub title: String,
    pub preview: String,
    pub tags: Vec<String>,
    pub feelings: Vec<String>,
    pub mood: Option<i8>,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryEncryptionState {
    Plain,
    EncryptedUnlocked,
    EncryptedLocked,
}

pub use create::{
    create_encrypted_entry, create_encrypted_entry_with_body,
    create_encrypted_entry_with_body_and_feelings, create_encrypted_entry_with_editor_and_feelings,
    create_entry, create_entry_with_body, create_entry_with_body_and_feelings,
    create_entry_with_editor_and_feelings, entry_template,
};
pub use edit::{
    edit_encrypted_entry, move_entry_to_trash, open_editor, open_editor_body_only,
    set_updated_at_now,
};
pub use paths::{entry_path, is_encrypted_entry_file, is_entry_file, is_plain_entry_file};
pub use read::{
    EntryPath, collect_entry_paths, read_entries, read_entry, read_entry_content_with_identity,
    read_entry_with_identity, scan_entries, scan_entries_with_identity,
};

pub(crate) use paths::entry_date_from_path;
