use crate::AppResult;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    pub name: String,
    pub path: PathBuf,
}

pub fn list_journals(root: &Path) -> AppResult<Vec<Journal>> {
    let mut journals = Vec::new();
    if !root.exists() {
        return Ok(journals);
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if is_hidden_name(&name) {
            continue;
        }

        journals.push(Journal {
            name,
            path: entry.path(),
        });
    }

    journals.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(journals)
}

pub fn create_journal(root: &Path, name: &str) -> AppResult<Journal> {
    let name = validate_journal_name(name)?;
    let path = root.join(&name);
    fs::create_dir_all(&path)?;
    Ok(Journal { name, path })
}

pub fn validate_journal_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("journal name cannot be empty".into());
    }
    if is_hidden_name(trimmed) || trimmed == "." || trimmed == ".." {
        return Err(format!("'{trimmed}' is a reserved journal name").into());
    }
    let path = Path::new(trimmed);
    if path.components().count() != 1 || trimmed.contains('/') || trimmed.contains('\\') {
        return Err("journal name must be a single folder name".into());
    }

    Ok(trimmed.to_string())
}

pub(crate) fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_journal_adds_immediate_directory() {
        let dir = tempdir().unwrap();

        let journal = create_journal(dir.path(), "personal").unwrap();

        assert_eq!(journal.name, "personal");
        assert!(dir.path().join("personal").is_dir());
    }

    #[test]
    fn create_journal_rejects_reserved_hidden_and_nested_names() {
        let dir = tempdir().unwrap();

        assert!(create_journal(dir.path(), ".trash").is_err());
        assert!(create_journal(dir.path(), ".hidden").is_err());
        assert!(create_journal(dir.path(), "nested/name").is_err());
        assert!(create_journal(dir.path(), "../outside").is_err());
        assert!(create_journal(dir.path(), "").is_err());
    }

    #[test]
    fn list_journals_ignores_files_and_hidden_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        fs::create_dir_all(dir.path().join(".trash")).unwrap();
        fs::create_dir_all(dir.path().join(".sync")).unwrap();
        fs::write(dir.path().join("notes.md"), "not a journal").unwrap();

        let journals = list_journals(dir.path()).unwrap();

        assert_eq!(journals.len(), 1);
        assert_eq!(journals[0].name, "work");
    }
}
