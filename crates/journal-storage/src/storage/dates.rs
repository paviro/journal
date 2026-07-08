use chrono::{DateTime, FixedOffset, NaiveDate};

use super::{Entry, entry_date_from_path};

/// Parse an RFC3339 timestamp, preserving its original offset.
pub fn parse_entry_timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

pub fn entry_group_date(entry: &Entry) -> Option<NaiveDate> {
    entry
        .created_time()
        .map(|timestamp| timestamp.date_naive())
        .or_else(|| entry_date_from_path(&entry.path))
}

fn format_date_human(date: NaiveDate) -> String {
    date.format("%A, %-d %B %Y").to_string()
}

pub fn entry_timestamp_label(entry: &Entry) -> String {
    entry
        .created_time()
        .map(|timestamp| {
            format!(
                "{}, {}",
                format_date_human(timestamp.date_naive()),
                timestamp.format("%H:%M")
            )
        })
        .or_else(|| entry_date_from_path(&entry.path).map(format_date_human))
        .unwrap_or_else(|| "Entry".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(created_at: Option<&str>, path: &str) -> Entry {
        Entry {
            id: "id".to_string(),
            journal: "work".to_string(),
            path: PathBuf::from(path),
            encryption_state: journal_core::EntryEncryptionState::Plain,
            created_at: created_at.map(journal_core::Timestamp::parse),
            edited_at: None,
            preview: String::new(),
            metadata: journal_core::Metadata::default(),
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

    #[test]
    fn timestamp_label_prefers_created_timestamp() {
        let entry = entry(Some("2026-07-01T10:23:00+02:00"), "work/2026-01-01/id.md");

        assert_eq!(
            entry_timestamp_label(&entry),
            "Wednesday, 1 July 2026, 10:23"
        );
    }
}
