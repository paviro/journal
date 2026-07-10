mod assets;
mod codec;
mod create;
mod edit;
mod paths;
mod read;

#[cfg(test)]
mod tests;

pub use journal_core::{Entry, EntryEncryptionState, EntryPath, ImportSource, Metadata, Timestamp};

pub use assets::{AssetFailure, AssetReport, sole_stored_image, stored_image_reference};
pub(crate) use assets::{ingest_and_cleanup_opts, resolve_entry_asset_path};
pub(crate) use codec::EntryCodec;
pub use create::{ImportedEntryDraft, create_entry, create_imported_entry};
pub use edit::{
    EditOutcome, delete_empty_entry, delete_journal, move_entry_to_trash, set_entry_body,
};
#[cfg(test)]
pub use paths::entry_path;
pub use paths::{entry_id, is_encrypted_entry_file, is_entry_file, is_plain_entry_file};
pub use read::{
    collect_entry_paths, read_entries, read_entry, read_entry_content, scan_entries,
    scan_import_sources,
};
