use std::path::Path;

use chrono::NaiveDate;

use crate::Entry;

/// The date an entry is grouped under: its creation timestamp when present,
/// otherwise the date encoded in its filename.
pub fn entry_group_date(entry: &Entry) -> Option<NaiveDate> {
    entry
        .created_time()
        .map(|timestamp| timestamp.date_naive())
        .or_else(|| entry_date_from_path(&entry.path))
}

/// Parse the leading `YYYY-MM-DD` of an entry filename stem into a date.
pub fn entry_date_from_path(path: &Path) -> Option<NaiveDate> {
    let stem = path.file_stem()?.to_str()?;
    let date = stem.get(..10)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntryEncryptionState, Metadata, Timestamp};
    use std::path::PathBuf;

    fn entry(created_at: Option<&str>, path: &str) -> Entry {
        Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from(path),
            encryption_state: EntryEncryptionState::Plain,
            created_at: created_at.map(Timestamp::parse),
            edited_at: None,
            preview: String::new(),
            metadata: Metadata::default(),
            location: None,
            import: None,
            content: String::new(),
            word_count: 0,
            search_haystack: String::new(),
        }
    }

    #[test]
    fn group_date_prefers_created_timestamp() {
        let entry = entry(Some("2026-07-01T10:23:00+02:00"), "work/2026-01-01/id.md");

        assert_eq!(
            entry_group_date(&entry),
            Some(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())
        );
    }

    #[test]
    fn group_date_falls_back_to_filename_date() {
        let entry = entry(None, "work/2026/07/01/2026-07-01T10-23-00-id.md");

        assert_eq!(
            entry_group_date(&entry),
            Some(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())
        );
    }
}
