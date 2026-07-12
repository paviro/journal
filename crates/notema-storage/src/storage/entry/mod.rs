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

pub(crate) use assets::resolve_entry_asset_path;
pub use assets::{AssetFailure, AssetReport, sole_stored_image, stored_image_reference};
pub(crate) use codec::EntryCodec;
pub(crate) use create::create_entry;
pub use create::{EntryAssetOptions, EntryCreateOutcome, EntryDraft};
pub use edit::{EditOutcome, EntryEdit, EntryEditOutcome};
pub(crate) use edit::{delete_empty_entry, delete_journal, move_entry_to_trash, save_entry_edit};
#[cfg(test)]
pub(super) use paths::entry_path;
pub use paths::{entry_id, is_entry_file};
pub(crate) use paths::{is_encrypted_entry_file, is_plain_entry_file, random_id};
pub(crate) use read::{
    collect_entry_paths, read_entries, read_entry, read_entry_content, scan_entries,
    scan_import_sources,
};
