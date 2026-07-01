use crate::AppResult;
use std::path::PathBuf;

use super::scan_entries;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub journal: String,
    pub title: String,
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScopeFilter<'a> {
    AllJournals,
    Journal(&'a str),
}

pub fn search_entries(
    root: &std::path::Path,
    query: &str,
    scope: SearchScopeFilter<'_>,
) -> AppResult<Vec<SearchHit>> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }

    let mut hits = Vec::new();
    for entry in scan_entries(root)? {
        if matches!(scope, SearchScopeFilter::Journal(journal) if entry.journal != journal) {
            continue;
        }

        if entry.content.to_lowercase().contains(&needle) {
            hits.push(SearchHit {
                path: entry.path,
                journal: entry.journal,
                title: entry.title,
                preview: entry.preview,
            });
        }
    }

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn search_matches_entries() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work").join("2026-07-01")).unwrap();
        fs::write(
            dir.path().join("work").join("2026-07-01").join("entry.md"),
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\n# Alpha\nneedle\n",
        )
        .unwrap();

        let hits = search_entries(dir.path(), "needle", SearchScopeFilter::AllJournals).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal, "work");
        assert_eq!(hits[0].title, "Alpha");
    }

    #[test]
    fn search_can_be_scoped_to_a_journal() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work").join("2026-07-01")).unwrap();
        fs::create_dir_all(dir.path().join("home").join("2026-07-01")).unwrap();
        fs::write(
            dir.path().join("work").join("2026-07-01").join("entry.md"),
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\n# Work\nneedle\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("home").join("2026-07-01").join("entry.md"),
            "---\ncreated_at: \"2026-07-01T10:00:00+02:00\"\n---\n\n# Home\nneedle\n",
        )
        .unwrap();

        let hits =
            search_entries(dir.path(), "needle", SearchScopeFilter::Journal("work")).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal, "work");
        assert_eq!(hits[0].title, "Work");
    }
}
