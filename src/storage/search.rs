use crate::AppResult;
use std::path::PathBuf;

use super::scan_entries;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub label: String,
    pub preview: String,
}

pub fn search_all(root: &std::path::Path, query: &str) -> AppResult<Vec<SearchHit>> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }

    let mut hits = Vec::new();
    for entry in scan_entries(root)? {
        if entry.content.to_lowercase().contains(&needle) {
            hits.push(SearchHit {
                path: entry.path,
                label: format!("{}/{}", entry.journal, entry.title),
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

        let hits = search_all(dir.path(), "needle").unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "work/Alpha");
    }
}
