pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub mod entry;
pub mod feelings;
pub mod markdown;
pub mod search;

pub use entry::{
    Entry, EntryEncryptionState, EntryPath, MOOD_RANGE, Metadata, MetadataField, SearchHit,
    SearchScopeFilter,
};
pub use search::search_loaded_entries;
