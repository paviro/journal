pub type JournalResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub type AppResult<T> = JournalResult<T>;

pub mod entry;
pub mod feelings;
pub mod search;

pub use entry::{
    Entry, EntryEncryptionState, EntryMetadata, EntryPath, SearchHit, SearchScopeFilter,
};
pub use search::search_loaded_entries;
