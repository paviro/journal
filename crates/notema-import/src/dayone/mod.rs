//! Day One JSON export → store entries.
//!
//! [`model`] deserializes the export. Each entry's body is produced by one of
//! two paths — the structured [`richtext`] renderer when the entry carries a
//! clean `richText`, else the [`text`] cleanup of the lossy `text` field — and
//! both converge on `dayone-moment://` image references that the shared
//! [`moments`] resolver rewrites against the entry's on-disk media. The
//! orchestration lives in [`crate::import_dayone`].

pub(crate) mod model;
pub(crate) mod moments;
pub(crate) mod richtext;
pub(crate) mod text;
