use chrono::{DateTime, Local};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub journal: String,
    pub path: PathBuf,
    pub encryption_state: EntryEncryptionState,
    pub created_at: Option<String>,
    /// `created_at` parsed once at load into a real timestamp, so the grouping,
    /// label, and stats paths never re-run `DateTime::parse_from_rfc3339` per
    /// call. `None` when `created_at` is missing or unparseable (callers then
    /// fall back to the filename date).
    pub created: Option<DateTime<Local>>,
    pub updated_at: Option<String>,
    pub preview: String,
    pub tags: Vec<String>,
    pub people: Vec<String>,
    pub activities: Vec<String>,
    pub feelings: Vec<String>,
    pub mood: Option<i8>,
    /// Provenance of an imported entry, e.g. `"dayone:<UUID>"`. `None` for
    /// entries created directly in the app. Used to skip re-importing and as an
    /// anchor for back-filling richer metadata once the format supports it.
    pub import_id: Option<String>,
    pub content: String,
    /// Word count of `content`, computed once at load so the entry-list row
    /// builder never has to tokenize the full body on the render path.
    ///
    /// Derived from `content`: any code that mutates `content` in memory must
    /// recompute this. Today the load path is the only writer — all edits go
    /// through disk and are re-read — so no in-place update path exists yet.
    pub word_count: usize,
    /// Body + every metadata value merged into one string, built once at load so
    /// a whole-corpus fuzzy search never rebuilds the haystack per entry per
    /// keystroke.
    ///
    /// Derived from `content`, `tags`, `people`, `activities`, and `feelings`:
    /// any code that mutates one of those in memory must call
    /// [`Entry::rebuild_search_haystack`] afterward. Today the load path
    /// (`build_search_haystack`) is the only writer — all edits go through disk
    /// and are re-read — so `rebuild_search_haystack` has no production caller
    /// yet; it exists to keep the invariant restorable if one is added.
    pub search_haystack: String,
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

    /// Recompute [`Entry::search_haystack`] from the current body and metadata.
    /// Call after mutating any of those fields so the precomputed haystack stays
    /// in sync (the load path builds it directly instead).
    pub fn rebuild_search_haystack(&mut self) {
        self.search_haystack = build_search_haystack(
            &self.content,
            &self.tags,
            &self.people,
            &self.activities,
            &self.feelings,
        );
    }
}

/// Merge the body and every metadata value into one space-separated string, the
/// haystack a prefix-less fuzzy query is scored against. Precomputed at load into
/// [`Entry::search_haystack`].
pub fn build_search_haystack(
    content: &str,
    tags: &[String],
    people: &[String],
    activities: &[String],
    feelings: &[String],
) -> String {
    let mut buf = String::with_capacity(content.len() + 16);
    buf.push_str(content);
    for value in tags.iter().chain(people).chain(activities).chain(feelings) {
        buf.push(' ');
        buf.push_str(value);
    }
    buf
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
