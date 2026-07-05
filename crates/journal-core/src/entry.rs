use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub journal: String,
    pub path: PathBuf,
    pub encryption_state: EntryEncryptionState,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub preview: String,
    pub tags: Vec<String>,
    pub people: Vec<String>,
    pub activities: Vec<String>,
    pub feelings: Vec<String>,
    pub mood: Option<i8>,
    pub content: String,
}

impl Entry {
    /// A non-empty label for the entry: the start of the preview, else the
    /// created timestamp, else the id.
    pub fn display_label(&self) -> String {
        let preview = self.preview.trim();
        if !preview.is_empty() {
            return preview.chars().take(80).collect();
        }
        self.created_at.clone().unwrap_or_else(|| self.id.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryEncryptionState {
    Plain,
    EncryptedUnlocked,
    EncryptedLocked,
}

#[derive(Clone, Copy)]
pub struct EntryMetadata<'a> {
    pub tags: &'a [String],
    pub people: &'a [String],
    pub activities: &'a [String],
    pub feelings: &'a [String],
    pub mood: Option<i8>,
}

pub struct EntryPath {
    pub journal: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub id: String,
    pub journal: String,
    pub created_at: Option<String>,
    pub title: String,
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScopeFilter<'a> {
    AllJournals,
    Journal(&'a str),
}
