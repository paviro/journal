use super::journal_metadata::JournalTheme;
use crate::{AppResult, StorageError};
use anyhow::bail;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A journal is archived by suffixing its directory name with this. The suffix
/// stays part of the journal's identity (`Journal::name`, `entry.journal`) so
/// entry lookups keep working; it is stripped only for display.
pub const ARCHIVED_SUFFIX: &str = ".archived";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    /// The raw directory name, including the `.archived` suffix when archived.
    pub name: String,
    pub path: PathBuf,
    pub archived: bool,
    /// Stable id from the journal's `.journal.toml` sidecar; a handle for
    /// machine-written references that must survive a rename. Empty only when the
    /// sidecar exists but couldn't be read.
    pub id: String,
    /// The journal's own theme, or `None` to follow the global theme.
    pub theme: Option<JournalTheme>,
}

impl Journal {
    /// The name to show the user: the raw name with any `.archived` suffix removed.
    pub fn display_name(&self) -> &str {
        journal_display_name(&self.name)
    }
}

/// The raw journal name with any `.archived` suffix stripped, for display.
pub fn journal_display_name(name: &str) -> &str {
    name.strip_suffix(ARCHIVED_SUFFIX).unwrap_or(name)
}

/// Whether a raw journal name denotes an archived journal.
pub fn is_archived_name(name: &str) -> bool {
    name.ends_with(ARCHIVED_SUFFIX)
}

pub(crate) fn list_journals(root: &Path) -> AppResult<Vec<Journal>> {
    let mut journals = discover_journals(root)?;
    initialize_journals(&mut journals);
    Ok(journals)
}

pub(crate) fn discover_journals(root: &Path) -> AppResult<Vec<Journal>> {
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

        let archived = is_archived_name(&name);
        let path = entry.path();
        let meta = super::journal_metadata::read_metadata(&path);
        journals.push(Journal {
            name,
            path,
            archived,
            id: meta.id,
            theme: meta.theme,
        });
    }

    // Active journals first (alphabetical), then archived (alphabetical) so the
    // Vec's index order is the display order and the active/archived boundary is
    // a single split point.
    journals.sort_by(|a, b| {
        a.archived
            .cmp(&b.archived)
            .then_with(|| a.display_name().cmp(b.display_name()))
    });
    Ok(journals)
}

pub(crate) fn initialize_journals(journals: &mut [Journal]) {
    for journal in journals {
        let meta = super::journal_metadata::read_or_init_metadata(&journal.path);
        journal.id = meta.id;
        journal.theme = meta.theme;
    }
}

/// Archive or unarchive a journal by renaming its directory to add or strip the
/// [`ARCHIVED_SUFFIX`]. Returns the journal in its new state. Errors if the
/// target directory already exists.
pub(crate) fn set_journal_archived(root: &Path, name: &str, archived: bool) -> AppResult<Journal> {
    let display = journal_display_name(name);
    let target_name = if archived {
        format!("{display}{ARCHIVED_SUFFIX}")
    } else {
        display.to_string()
    };

    let source = root.join(name);
    let target = root.join(&target_name);
    if target_name == name {
        // Already in the requested state; nothing to do.
        let meta = super::journal_metadata::read_or_init_metadata(&target);
        return Ok(Journal {
            name: target_name,
            path: target,
            archived,
            id: meta.id,
            theme: meta.theme,
        });
    }
    if target.exists() {
        return Err(StorageError::TargetExists {
            what: "journal archive destination",
            path: target,
        }
        .into());
    }

    // The sidecar rides along inside the folder, so the id/theme survive the
    // archive rename; re-read from the new location to populate the result.
    fs::rename(&source, &target)?;
    let meta = super::journal_metadata::read_or_init_metadata(&target);
    Ok(Journal {
        name: target_name,
        path: target,
        archived,
        id: meta.id,
        theme: meta.theme,
    })
}

pub(crate) fn create_journal(root: &Path, name: &str) -> AppResult<Journal> {
    let name = validate_journal_name(name)?;
    // The archived marker is a reserved suffix — never let a user create a journal
    // that would masquerade as archived. (Validation itself accepts the suffix so
    // that resolving an already-archived journal by name still works.)
    if is_archived_name(&name) {
        bail!("'{ARCHIVED_SUFFIX}' is a reserved journal-name suffix");
    }
    let path = root.join(&name);
    fs::create_dir_all(&path)?;
    // Reading the sidecar here backfills it, so a new journal gets its id at once.
    let meta = super::journal_metadata::read_or_init_metadata(&path);
    Ok(Journal {
        name,
        path,
        archived: false,
        id: meta.id,
        theme: meta.theme,
    })
}

pub(crate) fn validate_journal_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("journal name cannot be empty");
    }
    if is_hidden_name(trimmed) || trimmed == "." || trimmed == ".." {
        bail!("'{trimmed}' is a reserved journal name");
    }
    let path = Path::new(trimmed);
    if path.components().count() != 1 || trimmed.contains('/') || trimmed.contains('\\') {
        bail!("journal name must be a single folder name");
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
        assert!(create_journal(dir.path(), "personal.archived").is_err());
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
        assert!(!journals[0].archived);
    }

    #[test]
    fn list_journals_orders_active_before_archived_and_marks_them() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work")).unwrap();
        fs::create_dir_all(dir.path().join("personal")).unwrap();
        fs::create_dir_all(dir.path().join("old.archived")).unwrap();
        fs::create_dir_all(dir.path().join("ancient.archived")).unwrap();

        let journals = list_journals(dir.path()).unwrap();
        let summary: Vec<(&str, bool)> = journals
            .iter()
            .map(|j| (j.name.as_str(), j.archived))
            .collect();

        // Active first (alphabetical), then archived (alphabetical by display name).
        assert_eq!(
            summary,
            vec![
                ("personal", false),
                ("work", false),
                ("ancient.archived", true),
                ("old.archived", true),
            ]
        );
        assert_eq!(journals[2].display_name(), "ancient");
    }

    #[test]
    fn set_journal_archived_renames_both_ways() {
        let dir = tempdir().unwrap();
        create_journal(dir.path(), "personal").unwrap();

        let archived = set_journal_archived(dir.path(), "personal", true).unwrap();
        assert_eq!(archived.name, "personal.archived");
        assert!(archived.archived);
        assert!(dir.path().join("personal.archived").is_dir());
        assert!(!dir.path().join("personal").exists());

        let restored = set_journal_archived(dir.path(), "personal.archived", false).unwrap();
        assert_eq!(restored.name, "personal");
        assert!(!restored.archived);
        assert!(dir.path().join("personal").is_dir());
        assert!(!dir.path().join("personal.archived").exists());
    }

    #[test]
    fn create_journal_assigns_a_stable_id() {
        let dir = tempdir().unwrap();
        let created = create_journal(dir.path(), "personal").unwrap();
        assert!(!created.id.is_empty());
        assert_eq!(created.theme, None);
        // Listing returns the same persisted id.
        let listed = &list_journals(dir.path()).unwrap()[0];
        assert_eq!(listed.id, created.id);
    }

    #[test]
    fn list_journals_backfills_ids_for_pre_existing_folders() {
        let dir = tempdir().unwrap();
        // A folder made by hand (no sidecar), as an older journal would be.
        fs::create_dir_all(dir.path().join("work")).unwrap();
        let first = list_journals(dir.path()).unwrap().pop().unwrap();
        assert!(!first.id.is_empty());
        // The minted id is persisted and stable across listings.
        let second = list_journals(dir.path()).unwrap().pop().unwrap();
        assert_eq!(second.id, first.id);
    }

    #[test]
    fn archiving_preserves_the_id_and_theme() {
        let dir = tempdir().unwrap();
        let created = create_journal(dir.path(), "personal").unwrap();
        let theme = JournalTheme {
            name: "gameboy".to_string(),
            color_mode: Some("dark".to_string()),
            chrome: Some("flat".to_string()),
        };
        super::super::journal_metadata::set_theme(&created.path, Some(&theme)).unwrap();

        let archived = set_journal_archived(dir.path(), "personal", true).unwrap();
        assert_eq!(archived.id, created.id);
        assert_eq!(archived.theme, Some(theme));
    }

    #[test]
    fn set_journal_archived_errors_when_target_exists() {
        let dir = tempdir().unwrap();
        create_journal(dir.path(), "personal").unwrap();
        fs::create_dir_all(dir.path().join("personal.archived")).unwrap();

        assert!(set_journal_archived(dir.path(), "personal", true).is_err());
        // The source is left untouched on error.
        assert!(dir.path().join("personal").is_dir());
    }
}
