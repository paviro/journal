pub type AppResult<T> = anyhow::Result<T>;

pub mod dates;
pub mod entry;
pub mod feelings;
pub mod markdown;
pub mod paths;
pub mod search;

pub use dates::{entry_date_from_path, entry_group_date};
pub use entry::{
    AirQuality, Celestial, Entry, EntryEncryptionState, EntryPath, ImportSource, Location,
    MOOD_RANGE, Metadata, MetadataField, SearchHit, SearchScope, Timestamp, Weather,
};
pub use search::search_loaded_entries;
