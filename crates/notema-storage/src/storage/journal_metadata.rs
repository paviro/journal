//! The per-journal sidecar (`.journal.toml`): a small, app-managed TOML file
//! inside each journal folder carrying a stable id and an optional theme. It
//! syncs with the journal folder (unlike device-local config/state) and stays
//! plaintext even on an encrypted store — a theme name and a random id aren't
//! sensitive, matching the roster/pins being plaintext in `.age/`.

use super::random_id;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const JOURNAL_SCHEMA_VERSION: u32 = 1;
const JOURNAL_META_FILE: &str = ".journal.toml";
const JOURNAL_ID_LEN: usize = 8;

/// A journal's own theme, stored in its sidecar and saved/cleared as one unit.
/// The mode and chrome are plain strings so the storage format stays independent
/// of the UI's enums; a value another device doesn't recognize falls back to
/// that device's config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalTheme {
    /// Theme file name, without `.toml`.
    pub name: String,
    /// `"auto"` | `"dark"` | `"light"`; absent means the device's own setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_mode: Option<String>,
    /// `"default"` | `"flat"` | `"bordered"`; absent means the device's own
    /// setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome: Option<String>,
}

/// The parsed contents of a journal's `.journal.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JournalMetadata {
    /// Stable handle for machine-written references (e.g. `state.last_journal_id`),
    /// so they survive a folder rename or archive. Empty only when the sidecar
    /// exists but couldn't be read (corrupt / future schema) — treated as "no id".
    pub id: String,
    /// The journal's own theme, or `None` to follow the global theme.
    pub theme: Option<JournalTheme>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalMetaFile {
    schema_version: u32,
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme: Option<JournalTheme>,
}

/// The sidecar path for a journal directory.
pub(crate) fn metadata_path(journal_dir: &Path) -> PathBuf {
    journal_dir.join(JOURNAL_META_FILE)
}

/// Read a journal's metadata, minting and writing a fresh id when the sidecar
/// doesn't exist yet (backfill). Never fails: a corrupt or future-schema file is
/// left untouched and reported with no theme, and any other read error yields an
/// empty id without writing — so an unreadable sidecar can't break listing the
/// journals or get clobbered.
pub(crate) fn read_or_init_metadata(journal_dir: &Path) -> JournalMetadata {
    let path = metadata_path(journal_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_metadata(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // A write failure (read-only media) falls back to a session-only id.
            let id = random_id(JOURNAL_ID_LEN);
            let _ = write_metadata(journal_dir, &id, None);
            JournalMetadata { id, theme: None }
        }
        // Exists but unreadable (permissions, transient IO): don't overwrite it.
        Err(_) => JournalMetadata {
            id: String::new(),
            theme: None,
        },
    }
}

/// Read metadata without creating a missing sidecar. Used while inspecting a
/// folder that the user has not accepted yet.
pub(crate) fn read_metadata(journal_dir: &Path) -> JournalMetadata {
    std::fs::read_to_string(metadata_path(journal_dir)).map_or_else(
        |_| JournalMetadata {
            id: String::new(),
            theme: None,
        },
        |text| parse_metadata(&text),
    )
}

/// Set (or clear, with `None`) a journal's theme, preserving its id. Regenerates
/// a fresh valid document if the existing sidecar was unreadable.
pub(crate) fn set_theme(journal_dir: &Path, theme: Option<&JournalTheme>) -> crate::AppResult<()> {
    let current = read_or_init_metadata(journal_dir);
    let id = if current.id.is_empty() {
        random_id(JOURNAL_ID_LEN)
    } else {
        current.id
    };
    write_metadata(journal_dir, &id, theme.cloned())
}

fn write_metadata(
    journal_dir: &Path,
    id: &str,
    theme: Option<JournalTheme>,
) -> crate::AppResult<()> {
    let document = JournalMetaFile {
        schema_version: JOURNAL_SCHEMA_VERSION,
        id: id.to_string(),
        theme,
    };
    let text = toml::to_string_pretty(&document)?;
    notema_encryption::atomic_write(&metadata_path(journal_dir), text.as_bytes())?;
    Ok(())
}

/// Parse sidecar text. A v1 document is used as-is; anything else (unsupported
/// version, or unparseable) yields no theme and a best-effort id lifted from the
/// raw table, and is deliberately not rewritten.
fn parse_metadata(text: &str) -> JournalMetadata {
    // Pre-read the version from a lenient table first, so a future schema (which
    // deny_unknown_fields would reject) is recognized rather than treated as
    // corrupt, and we can still salvage its id.
    let raw: Option<toml::Table> = toml::from_str(text).ok();
    let version = raw
        .as_ref()
        .and_then(|table| table.get("schema_version"))
        .and_then(toml::Value::as_integer);

    if version == Some(i64::from(JOURNAL_SCHEMA_VERSION))
        && let Ok(parsed) = toml::from_str::<JournalMetaFile>(text)
    {
        return JournalMetadata {
            id: parsed.id,
            theme: parsed.theme,
        };
    }

    let id = raw
        .and_then(|table| {
            table
                .get("id")
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    JournalMetadata { id, theme: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(name: &str) -> JournalTheme {
        JournalTheme {
            name: name.to_string(),
            color_mode: Some("dark".to_string()),
            chrome: Some("flat".to_string()),
        }
    }

    #[test]
    fn missing_sidecar_is_backfilled_with_a_stable_id() {
        let dir = tempfile::tempdir().unwrap();
        let first = read_or_init_metadata(dir.path());
        assert!(!first.id.is_empty());
        assert_eq!(first.theme, None);
        assert!(metadata_path(dir.path()).exists());
        // A second read returns the same persisted id.
        let second = read_or_init_metadata(dir.path());
        assert_eq!(second.id, first.id);
    }

    #[test]
    fn set_and_clear_theme_preserves_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let id = read_or_init_metadata(dir.path()).id;
        set_theme(dir.path(), Some(&theme("gameboy"))).unwrap();
        let with = read_or_init_metadata(dir.path());
        assert_eq!(with.id, id);
        assert_eq!(with.theme, Some(theme("gameboy")));
        set_theme(dir.path(), None).unwrap();
        let without = read_or_init_metadata(dir.path());
        assert_eq!(without.id, id);
        assert_eq!(without.theme, None);
    }

    #[test]
    fn theme_without_mode_or_chrome_parses_with_none_fields() {
        let dir = tempfile::tempdir().unwrap();
        let id = read_or_init_metadata(dir.path()).id;
        std::fs::write(
            metadata_path(dir.path()),
            format!("schema_version = 1\nid = \"{id}\"\n\n[theme]\nname = \"gameboy\"\n"),
        )
        .unwrap();
        let meta = read_or_init_metadata(dir.path());
        assert_eq!(
            meta.theme,
            Some(JournalTheme {
                name: "gameboy".to_string(),
                color_mode: None,
                chrome: None,
            })
        );
    }

    #[test]
    fn corrupt_or_future_sidecar_is_left_untouched_without_a_theme() {
        let dir = tempfile::tempdir().unwrap();
        // Future schema: recognized, id salvaged, theme dropped, file untouched.
        let future =
            "schema_version = 2\nid = \"keepme00\"\nfuture = true\n\n[theme]\nname = \"gameboy\"\n";
        std::fs::write(metadata_path(dir.path()), future).unwrap();
        let meta = read_or_init_metadata(dir.path());
        assert_eq!(meta.id, "keepme00");
        assert_eq!(meta.theme, None);
        assert_eq!(
            std::fs::read_to_string(metadata_path(dir.path())).unwrap(),
            future,
            "a future-schema sidecar must not be rewritten"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_sidecar_is_not_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = metadata_path(dir.path());
        std::fs::write(&path, "schema_version = 1\nid = \"keepme00\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let meta = read_or_init_metadata(dir.path());
        assert_eq!(
            meta.id, "",
            "unreadable must report the empty (no-id) state"
        );
        assert_eq!(meta.theme, None);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "schema_version = 1\nid = \"keepme00\"\n",
            "an unreadable sidecar must not be rewritten"
        );
    }
}
