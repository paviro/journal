mod assets;
mod create;
mod edit;
mod paths;
mod read;

#[cfg(test)]
mod tests;

pub use journal_core::{Entry, EntryEncryptionState, EntryMetadata, EntryPath};

pub use assets::{AssetFailure, AssetReport, sole_stored_image, stored_image_reference};
pub(crate) use assets::{ingest_and_cleanup_opts, resolve_entry_asset_path};
#[cfg(test)]
pub use create::entry_template;
pub use create::{
    create_encrypted_entry_with_body_and_metadata,
    create_encrypted_imported_entry_with_body_and_metadata, create_entry_with_body_and_metadata,
    create_imported_entry_with_body_and_metadata,
};
pub use edit::{
    delete_empty_entry, delete_journal, edit_entry_body, move_entry_to_trash,
    write_encrypted_entry_content,
};
#[cfg(test)]
pub use paths::entry_path;
pub use paths::{entry_id, is_encrypted_entry_file, is_entry_file, is_plain_entry_file};
pub use read::{
    collect_entry_paths, read_entries, read_entry_content_with_identity, read_entry_with_identity,
    scan_entries_with_identity,
};
#[cfg(test)]
pub use read::{read_entry, scan_entries};

pub(crate) use paths::entry_date_from_path;
