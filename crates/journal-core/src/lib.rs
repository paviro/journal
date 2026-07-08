pub type AppResult<T> = anyhow::Result<T>;

pub mod entry;
pub mod feelings;
pub mod markdown;
pub mod search;

pub use entry::{
    Entry, EntryEncryptionState, EntryPath, ImportSource, Location, MOOD_RANGE, Metadata,
    MetadataField, SearchHit, SearchScope, Timestamp,
};
pub use search::search_loaded_entries;
