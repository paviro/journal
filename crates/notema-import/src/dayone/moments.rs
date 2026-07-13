//! Shared moment/media resolver — the convergence point of both body paths.
//!
//! Whichever producer built the body (the structured [`crate::dayone::richtext`]
//! renderer or the [`crate::dayone::text`] cleanup), it leaves image references
//! as `dayone-moment://<id>` links. [`MediaIndex`] maps an entry's declared media
//! to on-disk locations, and [`rewrite_moments`] resolves those references
//! against it: photos become local `![alt](<path>)` image embeds and
//! audio/video/pdf attachments become `[<Kind> attachment](<path>)` links — both
//! of which the store's asset ingestion copies in — while unknown ids are
//! recorded.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::model::{DayOneEntry, Moment};

/// Identifier → on-disk location for an entry's media, split by kind. Every map
/// holds only moments whose file was found on disk; the rest fall through to
/// `unresolved` during rewriting.
pub(crate) struct MediaIndex {
    pub photos: HashMap<String, PathBuf>,
    pub audio: HashMap<String, PathBuf>,
    pub video: HashMap<String, PathBuf>,
    pub pdf: HashMap<String, PathBuf>,
}

impl MediaIndex {
    pub(crate) fn build(entry: &DayOneEntry, media_root: &Path) -> Self {
        Self {
            photos: file_map(&entry.photos, media_root, "photos"),
            audio: file_map(&entry.audios, media_root, "audios"),
            video: file_map(&entry.videos, media_root, "videos"),
            pdf: file_map(&entry.pdf_attachments, media_root, "pdfs"),
        }
    }
}

/// Map each moment with a locatable file to its on-disk path.
fn file_map(moments: &[Moment], media_root: &Path, folder: &str) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    for moment in moments {
        if let Some(path) = resolve_moment_file(moment, media_root, folder) {
            map.insert(moment.identifier.clone(), path);
        }
    }
    map
}

/// Locate a moment's file as `<media_root>/<folder>/<md5>.<ext>`. The extension
/// is taken from `type`, then `format`; if neither names an existing file the
/// folder is scanned for a `<md5>.*` sibling — Day One stores audio as
/// `audios/<md5>.m4a` regardless of the `format` field. Returns a path only when
/// the file exists.
fn resolve_moment_file(moment: &Moment, media_root: &Path, folder: &str) -> Option<PathBuf> {
    let md5 = moment.md5.as_ref()?;
    let dir = media_root.join(folder);

    for ext in [moment.kind.as_deref(), moment.format.as_deref()]
        .into_iter()
        .flatten()
    {
        let candidate = dir.join(format!("{md5}.{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    for entry in fs::read_dir(&dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_stem().and_then(|stem| stem.to_str()) == Some(md5.as_str()) {
            return Some(path);
        }
    }
    None
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct MomentRewrite {
    pub body: String,
    pub attachment_paths: HashSet<PathBuf>,
    /// Moment identifiers referenced by the body but not found in any of the
    /// entry's media arrays (or whose file was missing on disk). Left unresolved
    /// and dropped from the body.
    pub unresolved: Vec<String>,
}

impl MomentRewrite {
    pub(crate) fn linked_attachments(&self) -> usize {
        self.attachment_paths.len()
    }
}

/// A generic `[<Kind> attachment](<path>)` link the store's asset ingestion
/// copies in. The label is intentionally kind-only (no filename) so imported
/// entries read consistently.
fn attachment_link(kind: &str, path: &Path) -> String {
    format!("[{kind} attachment]({})", path.display())
}

/// Rewrite `dayone-moment://` references in a Markdown body.
///
/// Photo moments become local `![alt](<absolute path>)` image embeds;
/// audio/video/pdf moments become `[<Kind> attachment](<absolute path>)` links.
/// Both point at real files the store's asset ingestion copies in. Moments with
/// no file on disk are dropped and recorded in `unresolved`. Non-moment images
/// (and any other Markdown) pass through untouched.
///
/// Classification is by identifier membership in the entry's media arrays, not
/// by the moment URL shape, which Day One has spelled inconsistently over time.
pub(crate) fn rewrite_moments(text: &str, media: &MediaIndex) -> MomentRewrite {
    let mut result = MomentRewrite::default();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(rel_start) = rest.find("![") {
        let (before, tag_start) = rest.split_at(rel_start);
        match parse_image(tag_start) {
            Some(image) if is_moment(image.target) => {
                out.push_str(before);
                let id = moment_identifier(image.target);
                if let Some(path) = media.photos.get(id) {
                    out.push_str(&format!("![{}]({})", image.alt, path.display()));
                } else if let Some(path) = media.audio.get(id) {
                    out.push_str(&attachment_link("Audio", path));
                    result.attachment_paths.insert(path.clone());
                } else if let Some(path) = media.video.get(id) {
                    out.push_str(&attachment_link("Video", path));
                    result.attachment_paths.insert(path.clone());
                } else if let Some(path) = media.pdf.get(id) {
                    out.push_str(&attachment_link("PDF", path));
                    result.attachment_paths.insert(path.clone());
                } else {
                    result.unresolved.push(id.to_string());
                }
                rest = &tag_start[image.len..];
            }
            // Not a moment we handle (regular image or malformed): emit the
            // `![` and keep scanning past it.
            _ => {
                out.push_str(before);
                out.push_str("![");
                rest = &tag_start[2..];
            }
        }
    }
    out.push_str(rest);

    result.body = out;
    result
}

struct ParsedImage<'a> {
    alt: &'a str,
    target: &'a str,
    /// Byte length of the whole `![alt](target)` starting at the `!`.
    len: usize,
}

/// Parse a `![alt](target)` starting at `s[0] == '!'`. Returns `None` if `s`
/// does not begin a well-formed image tag.
fn parse_image(s: &str) -> Option<ParsedImage<'_>> {
    let span = notema_domain::parse_inline_at(s)?;
    if !span.is_image {
        return None;
    }
    Some(ParsedImage {
        alt: &s[span.text],
        target: &s[span.target],
        len: span.span.end,
    })
}

fn is_moment(target: &str) -> bool {
    target.trim().starts_with("dayone-moment:")
}

/// Extract the moment identifier from a `dayone-moment:` target, tolerating both
/// `dayone-moment://<id>` and `dayone-moment:/audio/<id>` shapes by taking the
/// last path segment.
fn moment_identifier(target: &str) -> &str {
    let rest = target
        .trim()
        .strip_prefix("dayone-moment:")
        .unwrap_or(target)
        .trim_start_matches('/');
    rest.rsplit('/').next().unwrap_or(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(photos: HashMap<String, PathBuf>, audio: HashMap<String, PathBuf>) -> MediaIndex {
        MediaIndex {
            photos,
            audio,
            video: HashMap::new(),
            pdf: HashMap::new(),
        }
    }

    #[test]
    fn rewrites_photo_moment_to_local_path() {
        let mut photos = HashMap::new();
        photos.insert("PHOTO1".to_string(), PathBuf::from("/exp/photos/aaa.jpeg"));
        let index = media(photos, HashMap::new());
        let out = rewrite_moments("![](dayone-moment://PHOTO1)\n\nHi", &index);
        assert_eq!(out.body, "![](/exp/photos/aaa.jpeg)\n\nHi");
        assert_eq!(out.linked_attachments(), 0);
        assert!(out.unresolved.is_empty());
    }

    #[test]
    fn links_audio_moment_to_local_path() {
        let mut audio = HashMap::new();
        audio.insert("AUD1".to_string(), PathBuf::from("/exp/audios/aaa.m4a"));
        let index = media(HashMap::new(), audio);
        let out = rewrite_moments("Before ![](dayone-moment:/audio/AUD1) after", &index);
        assert_eq!(
            out.body,
            "Before [Audio attachment](/exp/audios/aaa.m4a) after"
        );
        assert_eq!(out.linked_attachments(), 1);
    }

    #[test]
    fn counts_repeated_attachment_path_once() {
        let mut audio = HashMap::new();
        audio.insert("AUD1".to_string(), PathBuf::from("/exp/audios/aaa.m4a"));
        let index = media(HashMap::new(), audio);

        let out = rewrite_moments(
            "![](dayone-moment://AUD1) and ![](dayone-moment://AUD1)",
            &index,
        );

        assert_eq!(out.linked_attachments(), 1);
    }

    #[test]
    fn unknown_moment_is_recorded_unresolved() {
        let index = media(HashMap::new(), HashMap::new());
        let out = rewrite_moments("![](dayone-moment://GONE)", &index);
        assert_eq!(out.unresolved, vec!["GONE".to_string()]);
        assert_eq!(out.body, "");
    }

    #[test]
    fn leaves_regular_images_untouched() {
        let index = media(HashMap::new(), HashMap::new());
        let body = "See ![a cat](cat.png) here";
        let out = rewrite_moments(body, &index);
        assert_eq!(out.body, body);
    }
}
