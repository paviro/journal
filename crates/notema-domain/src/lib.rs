#![forbid(unsafe_code)]

mod coordinates;
mod dates;
mod entry;
mod feelings;
mod markdown;

pub use coordinates::{CoordinateError, Coordinates};
pub use dates::{entry_date_from_path, entry_group_date};
pub use entry::{
    AirQuality, Celestial, Entry, EntryEncryptionState, EntryPath, ImportSource, Location,
    MOOD_RANGE, Metadata, MetadataField, SearchHit, SearchScope, Timestamp, Weather,
    build_search_haystack, normalize_for_search,
};
pub use feelings::{
    FEELING_GROUPS, Feeling, FeelingGroup, feelings, normalize_feeling, normalize_feelings,
    validate_feelings,
};
pub use markdown::{InlineSpan, parse_inline_at};
