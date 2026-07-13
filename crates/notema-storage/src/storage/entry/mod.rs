mod assets;
mod codec;
mod create;
mod edit;
mod paths;
mod read;

#[cfg(test)]
mod tests;

pub(crate) use notema_domain::{
    Entry, EntryEncryptionState, EntryPath, ImportSource, Metadata, Timestamp,
};

pub use assets::{
    AssetFailure, AssetReport, resolve_entry_asset_path, sole_stored_image, stored_asset_reference,
    stored_asset_reference_for,
};
pub(crate) use codec::EntryCodec;
pub use create::{EntryAssetOptions, EntryCreateOutcome, EntryDraft};
pub(crate) use create::{create_entry, create_entry_copy};
pub use edit::{EditOutcome, EntryEdit, EntryEditOutcome};
pub(crate) use edit::{
    delete_empty_entry, delete_journal, move_entry_to_trash, save_entry_edit,
    save_entry_edit_if_revision,
};
#[cfg(test)]
pub(super) use paths::entry_path;
pub use paths::{entry_id, is_entry_file};
pub(crate) use paths::{is_encrypted_entry_file, is_plain_entry_file, random_id};
#[cfg(test)]
use read::scan_entries;
pub(crate) use read::{
    collect_discovered_entries_with_progress, collect_entry_paths, read_entries,
    read_entries_with_progress, read_entry, read_entry_content, scan_import_sources,
};
