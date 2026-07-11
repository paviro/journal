//! Shared moment/media resolver — the convergence point of both body paths.
//!
//! Whichever producer built the body (the structured [`crate::dayone::richtext`]
//! renderer or the [`crate::dayone::text`] cleanup), it leaves image references
//! as `dayone-moment://<id>` links. [`MediaIndex`] maps an entry's declared media
//! to on-disk locations, and [`rewrite_moments`] resolves those references
//! against it: photos become local `![alt](<path>)` links the store can ingest,
//! non-image attachments are dropped and counted, and unknown ids are recorded.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::model::{DayOneEntry, Moment};

/// Identifier → on-disk location for an entry's media, split by kind. Photos map
/// to an absolute path (only files that exist); the other kinds are identifier
/// sets used to classify and count skipped attachments.
pub struct MediaIndex {
    pub photos: HashMap<String, PathBuf>,
    pub audio: HashSet<String>,
    pub video: HashSet<String>,
    pub pdf: HashSet<String>,
}

impl MediaIndex {
    pub fn build(entry: &DayOneEntry, media_root: &Path) -> Self {
        let mut photos = HashMap::new();
        for photo in &entry.photos {
            if let Some(path) = moment_path(photo, media_root, "photos")
                && path.is_file()
            {
                photos.insert(photo.identifier.clone(), path);
            }
        }
        Self {
            photos,
            audio: identifier_set(&entry.audios),
            video: identifier_set(&entry.videos),
            pdf: identifier_set(&entry.pdf_attachments),
        }
    }
}

fn identifier_set(moments: &[Moment]) -> HashSet<String> {
    moments.iter().map(|m| m.identifier.clone()).collect()
}

/// The on-disk path for a moment: `<media_root>/<folder>/<md5>.<type>`.
fn moment_path(moment: &Moment, media_root: &Path, folder: &str) -> Option<PathBuf> {
    let md5 = moment.md5.as_ref()?;
    let kind = moment.kind.as_ref()?;
    Some(media_root.join(folder).join(format!("{md5}.{kind}")))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MomentRewrite {
    pub body: String,
    pub skipped_audio: usize,
    pub skipped_video: usize,
    pub skipped_pdf: usize,
    /// Moment identifiers referenced by the body but not found in any of the
    /// entry's media arrays (or missing an md5/type). Left unresolved and
    /// dropped from the body.
    pub unresolved: Vec<String>,
}

impl MomentRewrite {
    pub fn skipped_attachments(&self) -> usize {
        self.skipped_audio + self.skipped_video + self.skipped_pdf
    }
}

/// Rewrite `dayone-moment://` references in a Markdown body.
///
/// Photo moments are rewritten to a local `![alt](<absolute path>)` link so the
/// store's asset ingestion copies them in. Audio/video/pdf moments have no
/// destination yet, so their references are removed and counted. Non-moment
/// images (and any other Markdown) pass through untouched.
///
/// Classification is by identifier membership in the entry's media arrays, not
/// by the moment URL shape, which Day One has spelled inconsistently over time.
pub fn rewrite_moments(text: &str, media: &MediaIndex) -> MomentRewrite {
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
                } else if media.audio.contains(id) {
                    result.skipped_audio += 1;
                } else if media.video.contains(id) {
                    result.skipped_video += 1;
                } else if media.pdf.contains(id) {
                    result.skipped_pdf += 1;
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
    let span = notema_core::markdown::parse_inline_at(s)?;
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

    fn media(photos: HashMap<String, PathBuf>, audio: HashSet<String>) -> MediaIndex {
        MediaIndex {
            photos,
            audio,
            video: HashSet::new(),
            pdf: HashSet::new(),
        }
    }

    #[test]
    fn rewrites_photo_moment_to_local_path() {
        let mut photos = HashMap::new();
        photos.insert("PHOTO1".to_string(), PathBuf::from("/exp/photos/aaa.jpeg"));
        let index = media(photos, HashSet::new());
        let out = rewrite_moments("![](dayone-moment://PHOTO1)\n\nHi", &index);
        assert_eq!(out.body, "![](/exp/photos/aaa.jpeg)\n\nHi");
        assert_eq!(out.skipped_attachments(), 0);
        assert!(out.unresolved.is_empty());
    }

    #[test]
    fn drops_and_counts_audio_moment() {
        let mut audio = HashSet::new();
        audio.insert("AUD1".to_string());
        let index = media(HashMap::new(), audio);
        let out = rewrite_moments("Before ![](dayone-moment:/audio/AUD1) after", &index);
        assert_eq!(out.body, "Before  after");
        assert_eq!(out.skipped_audio, 1);
    }

    #[test]
    fn unknown_moment_is_recorded_unresolved() {
        let index = media(HashMap::new(), HashSet::new());
        let out = rewrite_moments("![](dayone-moment://GONE)", &index);
        assert_eq!(out.unresolved, vec!["GONE".to_string()]);
        assert_eq!(out.body, "");
    }

    #[test]
    fn leaves_regular_images_untouched() {
        let index = media(HashMap::new(), HashSet::new());
        let body = "See ![a cat](cat.png) here";
        let out = rewrite_moments(body, &index);
        assert_eq!(out.body, body);
    }
}
