use chrono::{DateTime, Local, NaiveDate};
use std::path::Path;

use super::Entry;

pub(crate) fn parse_entry_timestamp(value: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Local))
}

pub(crate) fn entry_date_from_path(path: &Path) -> Option<NaiveDate> {
    let stem = path.file_stem()?.to_str()?;
    let date = stem.get(..10)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

pub(crate) fn entry_group_date(entry: &Entry) -> Option<NaiveDate> {
    entry
        .created_at
        .as_deref()
        .and_then(parse_entry_timestamp)
        .map(|timestamp| timestamp.date_naive())
        .or_else(|| entry_date_from_path(&entry.path))
}

pub(crate) fn entry_timestamp_label(entry: &Entry) -> String {
    entry
        .created_at
        .as_deref()
        .and_then(parse_entry_timestamp)
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M").to_string())
        .or_else(|| {
            entry_date_from_path(&entry.path).map(|date| date.format("%Y-%m-%d").to_string())
        })
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
            encryption_state: crate::storage::EntryEncryptionState::Plain,
            created_at: created_at.map(str::to_string),
            updated_at: None,
            title: "Title".to_string(),
            preview: String::new(),
            content: String::new(),
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

        assert_eq!(entry_timestamp_label(&entry), "2026-07-01 10:23");
    }
}
