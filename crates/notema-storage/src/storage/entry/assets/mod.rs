//! Per-entry assets.
//!
//! [`ingest_and_cleanup`] copies/downloads external images (local paths or
//! `http(s)` URLs, in `![alt](target)` tags or bare on their own line) and
//! non-image file attachments (existing local files in `[label](target)` links)
//! into the entry's sibling `<stem>.assets/` folder, age-encrypting when the
//! store is encrypted, and rewrites references to the stored copy. Assets no
//! longer referenced by the rewritten body are deleted.
//!
//! Stored references are always canonical markdown pointing inside the entry's
//! own asset folder — `![alt](<stem>.assets/<id>.<ext>)` for images,
//! `[label](<stem>.assets/<id>.<ext>)` for attachments (the file on disk carries
//! an extra `.age` suffix when encrypted) — so plaintext entries stay viewable in
//! external markdown tools.

mod net;

use super::paths::{entry_assets_dir, entry_assets_dir_name, random_id};
use crate::AppResult;
use anyhow::bail;
use net::{FetchError, fetch_source};
use notema_encryption::{self as crypto, KeyPaths};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{self, Cursor, Write},
    path::{Component, Path, PathBuf},
};

/// Length of the random id used as an asset's filename stem.
const ASSET_ID_LEN: usize = 8;
/// Bounded retry count when allocating a collision-free asset id.
const ASSET_ID_ATTEMPTS: usize = 32;

/// Supported raster image extensions (lowercase, no dot).
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AssetReport {
    /// Files copied/downloaded into the asset folder.
    pub stored: usize,
    /// Stored files referenced as attachments rather than image embeds.
    pub attachments_stored: usize,
    /// Orphaned assets deleted during cleanup.
    pub removed: usize,
    /// Sources that could not be ingested, tagged by cause so callers can tell a
    /// benign remote skip from a genuine failure without parsing message text.
    pub failed: Vec<AssetFailure>,
}

/// Why an external asset reference was not stored, carrying enough to report it.
#[derive(Debug, PartialEq, Eq)]
pub enum AssetFailure {
    /// A remote source deliberately not fetched (downloads disabled) or whose
    /// host was unreachable. Benign: the reference is kept, or replaced with the
    /// offline placeholder — not a real ingestion failure.
    RemoteUnavailable { source: String },
    /// A source that should have ingested but errored: missing local file,
    /// unsupported/undecodable image, or a write failure.
    Ingest { source: String, error: String },
    /// A local attachment that could not be read or stored.
    AttachmentIngest { source: String, error: String },
}

impl AssetReport {
    /// Stored files referenced as image embeds.
    pub fn images_stored(&self) -> usize {
        self.stored.saturating_sub(self.attachments_stored)
    }

    /// Image sources that were unavailable or failed ingestion.
    pub fn images_not_stored(&self) -> usize {
        self.failed
            .iter()
            .filter(|failure| !matches!(failure, AssetFailure::AttachmentIngest { .. }))
            .count()
    }

    /// Attachment sources that failed ingestion.
    pub fn attachments_not_stored(&self) -> usize {
        self.failed
            .iter()
            .filter(|failure| matches!(failure, AssetFailure::AttachmentIngest { .. }))
            .count()
    }

    pub fn is_noop(&self) -> bool {
        self.stored == 0 && self.removed == 0 && self.failed.is_empty()
    }
}

/// Ingest external image and attachment references, then delete orphaned assets.
///
/// `encryption` is `Some` when the store encrypts entries (assets get an `.age`
/// suffix and are age-encrypted); `download_remote` gates fetching `http(s)`
/// URLs. Returns the rewritten body only when it changed. Sources that fail to
/// fetch are skipped and recorded in the report rather than aborting.
#[cfg(test)]
pub(crate) fn ingest_and_cleanup(
    entry_path: &Path,
    body: &str,
    encryption: Option<&KeyPaths>,
    download_remote: bool,
) -> AppResult<(Option<String>, AssetReport)> {
    ingest_and_cleanup_opts(entry_path, body, encryption, download_remote, false)
}

/// Like [`ingest_and_cleanup`], but when `replace_offline` is set, external image
/// references that could not be ingested are replaced with an `[Offline Image]`
/// placeholder instead of being left in the body. Used by bulk import so dead
/// links don't linger as broken image tags.
pub(crate) fn ingest_and_cleanup_opts(
    entry_path: &Path,
    body: &str,
    encryption: Option<&KeyPaths>,
    download_remote: bool,
    replace_offline: bool,
) -> AppResult<(Option<String>, AssetReport)> {
    let (Some(assets_dir), Some(dir_name)) = (
        entry_assets_dir(entry_path),
        entry_assets_dir_name(entry_path),
    ) else {
        return Ok((None, AssetReport::default()));
    };

    let encryption = encryption
        .map(crypto::EncryptionRecipients::for_store)
        .transpose()?;
    let mut ctx = IngestContext {
        assets_dir: &assets_dir,
        dir_name: &dir_name,
        encryption,
        download_remote,
        replace_offline,
        asset_ids: existing_asset_ids(&assets_dir)?,
        stored_sources: HashMap::new(),
        report: AssetReport::default(),
    };

    let new_body = rewrite_body(body, &mut ctx);
    let changed = new_body != body;

    cleanup_orphans(&assets_dir, &dir_name, &new_body, &mut ctx.report)?;

    let report = ctx.report;
    Ok((changed.then_some(new_body), report))
}

/// Retarget canonical stored-asset references (image embeds and attachment
/// links) when an entry is copied. Text outside Markdown targets, including
/// fenced code, is left byte-for-byte intact.
pub(crate) fn retarget_stored_asset_links(
    body: &str,
    source_dir_name: &str,
    target_dir_name: &str,
) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    let mut lines = body.split('\n').peekable();
    while let Some(line) = lines.next() {
        if is_fence(line) {
            in_fence = !in_fence;
            push_line(&mut out, line, lines.peek().is_some());
            continue;
        }
        if in_fence {
            push_line(&mut out, line, lines.peek().is_some());
            continue;
        }

        let rewritten = retarget_line(line, source_dir_name, target_dir_name);
        push_line(&mut out, &rewritten, lines.peek().is_some());
    }
    out
}

/// Retarget every stored image embed then every stored attachment link on a
/// single line, leaving external and already-mismatched targets untouched.
fn retarget_line(line: &str, source_dir_name: &str, target_dir_name: &str) -> String {
    let mut images = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(image) = next_markdown_image(rest) {
        images.push_str(&rest[..image.target_start]);
        images.push_str(&retarget_target(
            &rest[image.target_range()],
            source_dir_name,
            target_dir_name,
        ));
        rest = &rest[image.target_end..];
    }
    images.push_str(rest);

    let mut out = String::with_capacity(images.len());
    let mut rest = images.as_str();
    while let Some(link) = next_markdown_link(rest) {
        out.push_str(&rest[..link.target_start]);
        out.push_str(&retarget_target(
            &rest[link.target_range()],
            source_dir_name,
            target_dir_name,
        ));
        rest = &rest[link.target_end..];
    }
    out.push_str(rest);
    out
}

/// Rewrite a single target to the new asset folder when it is a canonical
/// reference into `source_dir_name`; otherwise return it unchanged.
fn retarget_target(target: &str, source_dir_name: &str, target_dir_name: &str) -> String {
    match stored_asset_reference(target, source_dir_name) {
        Some(reference) => format!("{target_dir_name}/{}", reference.file_name),
        None => target.to_string(),
    }
}

struct IngestContext<'a> {
    assets_dir: &'a Path,
    dir_name: &'a str,
    encryption: Option<crypto::EncryptionRecipients>,
    download_remote: bool,
    replace_offline: bool,
    asset_ids: HashSet<String>,
    stored_sources: HashMap<String, String>,
    report: AssetReport,
}

/// Placeholder substituted for an image that could not be ingested when
/// `replace_offline` is set.
const OFFLINE_IMAGE_PLACEHOLDER: &str = "[Offline Image]";

/// Rewrite a body line by line, ingesting external image references. Code
/// fences are passed through untouched.
fn rewrite_body(body: &str, ctx: &mut IngestContext<'_>) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;

    let mut lines = body.split('\n').peekable();
    while let Some(line) = lines.next() {
        if is_fence(line) {
            in_fence = !in_fence;
            push_line(&mut out, line, lines.peek().is_some());
            continue;
        }
        if in_fence {
            push_line(&mut out, line, lines.peek().is_some());
            continue;
        }

        let rewritten = rewrite_markdown_images(line, ctx);
        let rewritten = match rewrite_bare_line(&rewritten, ctx) {
            Some(replacement) => replacement,
            None => rewritten,
        };
        let rewritten = rewrite_markdown_links(&rewritten, ctx);
        push_line(&mut out, &rewritten, lines.peek().is_some());
    }

    out
}

fn push_line(out: &mut String, line: &str, more: bool) {
    out.push_str(line);
    if more {
        out.push('\n');
    }
}

fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// Replace every external `![alt](target)` in a line with a canonical stored
/// reference.
fn rewrite_markdown_images(line: &str, ctx: &mut IngestContext<'_>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(image) = next_markdown_image(rest) {
        out.push_str(&rest[..image.start]);
        let target = rest[image.target_range()].trim();
        if is_external_target(target, ctx.dir_name) {
            match store_source(target, image.alt(rest), ctx) {
                Some(link) => out.push_str(&link),
                None if ctx.replace_offline => out.push_str(OFFLINE_IMAGE_PLACEHOLDER),
                None => out.push_str(&rest[image.start..image.end]),
            }
        } else {
            out.push_str(&rest[image.start..image.end]);
        }
        rest = &rest[image.end..];
    }

    out.push_str(rest);
    out
}

/// If the whole trimmed line is a single bare path (or image URL), ingest it:
/// image sources become `![](…)` embeds, any other existing local file becomes a
/// `[<filename>](…)` attachment link. This is what makes dragging a file onto the
/// terminal — which pastes its path — attach it.
fn rewrite_bare_line(line: &str, ctx: &mut IngestContext<'_>) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Interpret the line the way a shell would (unquote, unescape `\ ` → ` `):
    // dragging or pasting a path into a terminal escapes spaces and other
    // special characters, e.g. `/a/IMG\ 2.jpeg`.
    let source = if is_url(trimmed) {
        trimmed.to_string()
    } else {
        unescape_shell_path(trimmed)
    };
    // Rely on `is_file()` (not a whitespace heuristic) to reject prose: a real
    // path may contain spaces, e.g. `.../Photos Library.photoslibrary/foo.jpeg`.
    if !is_external_target(&source, ctx.dir_name) {
        return None;
    }

    let indent = &line[..line.len() - line.trim_start().len()];
    if looks_like_image_source(&source) {
        return match store_source(&source, "", ctx) {
            Some(link) => Some(format!("{indent}{link}")),
            None if ctx.replace_offline => Some(format!("{indent}{OFFLINE_IMAGE_PLACEHOLDER}")),
            None => None,
        };
    }
    // A bare non-image line only attaches an existing *local* file; a remote URL
    // to some other file type is left as prose (attachments are never fetched).
    if is_url(&source) {
        return None;
    }
    let label = bare_attachment_label(&source);
    store_file_source(&source, &label, ctx).map(|link| format!("{indent}{link}"))
}

/// A human label for a bare-pasted attachment: the source's file name (without
/// any Markdown-breaking `[`/`]`), falling back to `attachment`.
fn bare_attachment_label(source: &str) -> String {
    Path::new(source)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.replace(['[', ']'], ""))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "attachment".to_string())
}

/// Replace every external file `[label](target)` in a line with a canonical
/// stored attachment reference. Only existing local files are ingested; URLs,
/// `#anchors`, `data:` URIs, and references already inside the asset folder pass
/// through untouched — attachments are never downloaded from remote hosts.
fn rewrite_markdown_links(line: &str, ctx: &mut IngestContext<'_>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(link) = next_markdown_link(rest) {
        out.push_str(&rest[..link.start]);
        let target = rest[link.target_range()].trim();
        if !is_url(target) && is_external_target(target, ctx.dir_name) {
            match store_file_source(target, link.text(rest), ctx) {
                Some(replacement) => out.push_str(&replacement),
                None => out.push_str(&rest[link.start..link.end]),
            }
        } else {
            out.push_str(&rest[link.start..link.end]);
        }
        rest = &rest[link.end..];
    }

    out.push_str(rest);
    out
}

/// Copy a local file into the asset folder (encrypted when configured) and
/// return the canonical `[label](<dir>/<file>)` link. Identical sources are
/// stored once and reused. Returns `None` on failure, recording it in the
/// report.
fn store_file_source(source: &str, label: &str, ctx: &mut IngestContext<'_>) -> Option<String> {
    if let Some(file_name) = ctx.stored_sources.get(source) {
        return Some(markdown_link(label, ctx.dir_name, file_name));
    }

    let ext = attachment_extension(source);
    store_asset(
        source,
        AssetReference::Attachment(label),
        AssetData::File(expand_user(source)),
        &ext,
        ctx,
    )
}

fn markdown_link(label: &str, dir_name: &str, file_name: &str) -> String {
    format!("[{label}]({dir_name}/{file_name})")
}

/// The lowercased file extension of an attachment source, defaulting to `bin`
/// when the path carries none.
fn attachment_extension(source: &str) -> String {
    extension_of(source).unwrap_or_else(|| "bin".to_string())
}

/// Fetch a source, store it in the asset folder (encrypted when configured),
/// and return the canonical reference. Identical sources are stored once and
/// reused. Returns `None` on failure, recording it in the report.
fn store_source(source: &str, alt: &str, ctx: &mut IngestContext<'_>) -> Option<String> {
    if let Some(file_name) = ctx.stored_sources.get(source) {
        return Some(markdown_image(alt, ctx.dir_name, file_name));
    }

    let (bytes, ext) = match fetch_source(source, ctx.download_remote) {
        Ok(value) => value,
        Err(FetchError::RemoteUnavailable) => {
            ctx.report.failed.push(AssetFailure::RemoteUnavailable {
                source: source.to_string(),
            });
            return None;
        }
        Err(FetchError::Ingest(error)) => {
            ctx.report.failed.push(AssetFailure::Ingest {
                source: source.to_string(),
                error,
            });
            return None;
        }
    };

    store_asset(
        source,
        AssetReference::Image(alt),
        AssetData::Bytes(bytes),
        &ext,
        ctx,
    )
}

fn markdown_image(alt: &str, dir_name: &str, file_name: &str) -> String {
    format!("![{alt}]({dir_name}/{file_name})")
}

enum AssetReference<'a> {
    Image(&'a str),
    Attachment(&'a str),
}

impl AssetReference<'_> {
    fn render(&self, dir_name: &str, file_name: &str) -> String {
        match self {
            Self::Image(alt) => markdown_image(alt, dir_name, file_name),
            Self::Attachment(label) => markdown_link(label, dir_name, file_name),
        }
    }

    fn failure(&self, source: String, error: String) -> AssetFailure {
        match self {
            Self::Image(_) => AssetFailure::Ingest { source, error },
            Self::Attachment(_) => AssetFailure::AttachmentIngest { source, error },
        }
    }

    fn is_attachment(&self) -> bool {
        matches!(self, Self::Attachment(_))
    }
}

enum AssetData {
    Bytes(Vec<u8>),
    File(PathBuf),
}

impl AssetData {
    fn write_to(
        &self,
        output: &mut fs::File,
        encryption: Option<&crypto::EncryptionRecipients>,
    ) -> AppResult<()> {
        match (self, encryption) {
            (Self::Bytes(bytes), Some(recipients)) => {
                recipients.encrypt_reader(Cursor::new(bytes), output)?;
            }
            (Self::File(path), Some(recipients)) => {
                recipients.encrypt_reader(fs::File::open(path)?, output)?;
            }
            (Self::Bytes(bytes), None) => output.write_all(bytes)?,
            (Self::File(path), None) => {
                io::copy(&mut fs::File::open(path)?, output)?;
            }
        }
        Ok(())
    }
}

fn store_asset(
    source: &str,
    reference: AssetReference<'_>,
    data: AssetData,
    ext: &str,
    ctx: &mut IngestContext<'_>,
) -> Option<String> {
    match write_asset(ctx, &data, ext) {
        Ok(file_name) => {
            ctx.report.stored += 1;
            if reference.is_attachment() {
                ctx.report.attachments_stored += 1;
            }
            ctx.stored_sources
                .insert(source.to_string(), file_name.clone());
            Some(reference.render(ctx.dir_name, &file_name))
        }
        Err(error) => {
            ctx.report
                .failed
                .push(reference.failure(source.to_string(), error.to_string()));
            None
        }
    }
}

/// Write asset data under a collision-free random id. Encrypted files gain an
/// on-disk `.age` suffix while body references stay unchanged.
fn write_asset(ctx: &mut IngestContext<'_>, data: &AssetData, ext: &str) -> AppResult<String> {
    fs::create_dir_all(ctx.assets_dir)?;

    for _ in 0..ASSET_ID_ATTEMPTS {
        let id = random_id(ASSET_ID_LEN);
        if !ctx.asset_ids.insert(id.clone()) {
            continue;
        }
        let link_name = format!("{id}.{ext}");
        let disk_name = match ctx.encryption {
            Some(_) => format!("{link_name}.age"),
            None => link_name.clone(),
        };
        let path = ctx.assets_dir.join(&disk_name);
        let mut output = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        if let Err(error) = data.write_to(&mut output, ctx.encryption.as_ref()) {
            drop(output);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        return Ok(link_name);
    }

    bail!("could not allocate a unique asset id")
}

fn existing_asset_ids(assets_dir: &Path) -> AppResult<HashSet<String>> {
    let mut ids = HashSet::new();
    if !assets_dir.exists() {
        return Ok(ids);
    }
    for item in fs::read_dir(assets_dir)? {
        let item = item?;
        if let Some(name) = item.file_name().to_str()
            && let Some((id, _)) = name.split_once('.')
        {
            ids.insert(id.to_string());
        }
    }
    Ok(ids)
}

/// Delete assets in the folder not referenced by any in-folder link in `body`.
fn cleanup_orphans(
    assets_dir: &Path,
    dir_name: &str,
    body: &str,
    report: &mut AssetReport,
) -> AppResult<()> {
    if !assets_dir.exists() {
        return Ok(());
    }

    let referenced = referenced_asset_files(body, dir_name);
    let mut remaining = 0usize;
    for item in fs::read_dir(assets_dir)? {
        let item = item?;
        if !item.file_type()?.is_file() {
            remaining += 1;
            continue;
        }
        // Body links are clean, but the file may carry a `.age` suffix — compare
        // by the clean key so a referenced encrypted asset isn't seen as orphaned.
        let name = item.file_name().to_string_lossy().to_string();
        let key = name.strip_suffix(".age").unwrap_or(&name);
        if referenced.contains(key) {
            remaining += 1;
        } else {
            fs::remove_file(item.path())?;
            report.removed += 1;
        }
    }

    if remaining == 0 {
        let _ = fs::remove_dir(assets_dir);
    }

    Ok(())
}

/// Collect the file names referenced by canonical in-folder references — both
/// `![...](<dir_name>/<file>)` image embeds and `[...](<dir_name>/<file>)`
/// attachment links — so neither kind is pruned as an orphan.
fn referenced_asset_files(body: &str, dir_name: &str) -> HashSet<String> {
    let mut files = HashSet::new();
    let mut rest = body;
    while let Some(image) = next_markdown_image(rest) {
        let target = rest[image.target_range()].trim();
        if let Some(reference) = stored_asset_reference(target, dir_name) {
            files.insert(reference.file_name);
        }
        rest = &rest[image.end..];
    }
    let mut rest = body;
    while let Some(link) = next_markdown_link(rest) {
        let target = rest[link.target_range()].trim();
        if let Some(reference) = stored_asset_reference(target, dir_name) {
            files.insert(reference.file_name);
        }
        rest = &rest[link.end..];
    }
    files
}

/// A canonical asset reference (image or attachment) inside an entry's own asset
/// folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAssetReference {
    pub file_name: String,
}

/// Parse the exact stored form `<entry-id>.assets/<file>`. Rejects anything
/// absolute, nested, traversal-based, external, or pointing at a different
/// assets directory. Extension-agnostic — it matches images and attachments
/// alike.
pub fn stored_asset_reference(target: &str, dir_name: &str) -> Option<StoredAssetReference> {
    if target.is_empty()
        || is_url(target)
        || target.starts_with('/')
        || target.starts_with('\\')
        || target.contains('\\')
    {
        return None;
    }

    let mut components = Path::new(target).components();
    let Some(Component::Normal(dir)) = components.next() else {
        return None;
    };
    if dir != dir_name {
        return None;
    }
    let Some(Component::Normal(file)) = components.next() else {
        return None;
    };
    if components.next().is_some() {
        return None;
    }
    let file_name = file.to_str()?;
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        return None;
    }
    Some(StoredAssetReference {
        file_name: file_name.to_string(),
    })
}

/// The stored file name if `target` is a canonical reference inside
/// `entry_path`'s own asset folder, else `None`. Pure string check (no
/// filesystem access) so callers on the render hot path can use it freely.
pub fn stored_asset_reference_for(entry_path: &Path, target: &str) -> Option<String> {
    let dir_name = entry_assets_dir_name(entry_path)?;
    stored_asset_reference(target, &dir_name).map(|reference| reference.file_name)
}

/// If `line` (ignoring surrounding whitespace) is exactly one markdown image
/// pointing inside `entry_path`'s own `<stem>.assets/` folder, return its alt
/// text and stored file name; any other text or a second image rejects it.
///
/// Shared by the entry-view labels and the fullscreen viewer so an image's
/// position (and its `Image N` number) is identical everywhere.
pub fn sole_stored_image(line: &str, entry_path: &Path) -> Option<(String, String)> {
    let dir_name = entry_assets_dir_name(entry_path)?;
    let trimmed = line.trim();
    let image = next_markdown_image(trimmed)?;
    if image.start != 0 || image.end != trimmed.len() {
        return None;
    }
    let target = trimmed[image.target_range()].trim();
    let reference = stored_asset_reference(target, &dir_name)?;
    Some((image.alt(trimmed).to_string(), reference.file_name))
}

/// Resolve a canonical stored asset name to an absolute path, rejecting
/// symlinks and any file that escapes the entry's own asset folder.
pub fn resolve_entry_asset_path(entry_path: &Path, file_name: &str) -> AppResult<Option<PathBuf>> {
    let Some(dir_name) = entry_assets_dir_name(entry_path) else {
        return Ok(None);
    };
    if stored_asset_reference(&format!("{dir_name}/{file_name}"), &dir_name).is_none() {
        return Ok(None);
    }

    let Some(assets_dir) = entry_assets_dir(entry_path) else {
        return Ok(None);
    };

    // A body link is always clean (`<id>.<ext>`); the file on disk carries a
    // `.age` suffix when encrypted. Try the plaintext name, then the encrypted
    // sibling.
    for candidate in asset_name_candidates(file_name) {
        if let Some(path) = resolve_regular_file(&assets_dir, &candidate)? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// The on-disk names a clean reference `<id>.<ext>` might map to: the plaintext
/// file itself, or its encrypted `.age` sibling.
fn asset_name_candidates(file_name: &str) -> [String; 2] {
    [file_name.to_string(), format!("{file_name}.age")]
}

/// Resolve `file_name` in `assets_dir` to an absolute path if it's a regular
/// file that doesn't escape the folder (rejecting symlinks and traversal).
fn resolve_regular_file(assets_dir: &Path, file_name: &str) -> AppResult<Option<PathBuf>> {
    let path = assets_dir.join(file_name);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !meta.file_type().is_file() || meta.file_type().is_symlink() {
        return Ok(None);
    }

    let assets_dir = fs::canonicalize(assets_dir)?;
    let path = fs::canonicalize(&path)?;
    if !path.starts_with(&assets_dir) {
        return Ok(None);
    }
    Ok(Some(path))
}

/// A located `![alt](target)` span.
struct MarkdownImage {
    start: usize,
    end: usize,
    alt_start: usize,
    alt_end: usize,
    target_start: usize,
    target_end: usize,
}

impl MarkdownImage {
    fn alt<'a>(&self, source: &'a str) -> &'a str {
        &source[self.alt_start..self.alt_end]
    }

    fn target_range(&self) -> std::ops::Range<usize> {
        self.target_start..self.target_end
    }
}

/// Find the next `![alt](target)` in `source` (no nested parens in target).
fn next_markdown_image(source: &str) -> Option<MarkdownImage> {
    // First `![` immediately followed by a parenthesized target wins.
    let mut base = 0;
    loop {
        let start = base + source[base..].find("![")?;
        if let Some(span) = notema_domain::parse_inline_at(&source[start..]) {
            return Some(MarkdownImage {
                start,
                end: start + span.span.end,
                alt_start: start + span.text.start,
                alt_end: start + span.text.end,
                target_start: start + span.target.start,
                target_end: start + span.target.end,
            });
        }
        base = start + 2;
    }
}

/// A located `[text](target)` link span (never an image).
struct MarkdownLink {
    start: usize,
    end: usize,
    text_start: usize,
    text_end: usize,
    target_start: usize,
    target_end: usize,
}

impl MarkdownLink {
    fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.text_start..self.text_end]
    }

    fn target_range(&self) -> std::ops::Range<usize> {
        self.target_start..self.target_end
    }
}

/// Find the next `[text](target)` link in `source`. A `[` preceded by `!` is an
/// image marker and is skipped so it stays the image pass's responsibility.
fn next_markdown_link(source: &str) -> Option<MarkdownLink> {
    let bytes = source.as_bytes();
    let mut base = 0;
    loop {
        let start = base + source[base..].find('[')?;
        if start > 0 && bytes[start - 1] == b'!' {
            base = start + 1;
            continue;
        }
        if let Some(span) = notema_domain::parse_inline_at(&source[start..])
            && !span.is_image
        {
            return Some(MarkdownLink {
                start,
                end: start + span.span.end,
                text_start: start + span.text.start,
                text_end: start + span.text.end,
                target_start: start + span.target.start,
                target_end: start + span.target.end,
            });
        }
        base = start + 1;
    }
}

fn is_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// Strip the query/fragment from a URL, leaving the path portion.
fn url_path(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..end]
}

/// True when a target should be ingested: a URL, or an existing local file not
/// already inside this entry's asset folder. `data:` URIs and internal
/// references are left untouched.
fn is_external_target(target: &str, dir_name: &str) -> bool {
    if target.is_empty() || target.starts_with("data:") {
        return false;
    }
    if is_url(target) {
        return true;
    }
    if target.starts_with(&format!("{dir_name}/")) {
        return false;
    }
    expand_user(target).is_file()
}

/// Whether a bare source looks like an image by its extension.
fn looks_like_image_source(source: &str) -> bool {
    let path = if is_url(source) {
        url_path(source)
    } else {
        source
    };
    extension_of(path).is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// Resolve the image extension from the source name, falling back to sniffing
/// magic bytes.
fn image_extension(name: &str, bytes: &[u8]) -> Option<String> {
    if let Some(ext) = extension_of(name)
        && IMAGE_EXTENSIONS.contains(&ext.as_str())
    {
        return Some(if ext == "jpeg" {
            "jpg".to_string()
        } else {
            ext
        });
    }
    sniff_extension(bytes).map(str::to_string)
}

fn extension_of(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

/// Identify a supported image format from its magic bytes.
fn sniff_extension(bytes: &[u8]) -> Option<&'static str> {
    use image::ImageFormat;
    match image::guess_format(bytes).ok()? {
        ImageFormat::Png => Some("png"),
        ImageFormat::Jpeg => Some("jpg"),
        ImageFormat::Gif => Some("gif"),
        ImageFormat::WebP => Some("webp"),
        ImageFormat::Bmp => Some("bmp"),
        _ => None,
    }
}

/// Interpret a pasted/dragged path the way a shell would: strip a single layer
/// of surrounding quotes and remove backslash escapes (`\ ` → ` `, `\(` → `(`,
/// …). Terminals add these when a path with spaces or special characters is
/// dragged in. On Unix a backslash is never a path separator, so a lone `\x`
/// collapses to `x`.
fn unescape_shell_path(raw: &str) -> String {
    let inner = if raw.len() >= 2
        && ((raw.starts_with('\'') && raw.ends_with('\''))
            || (raw.starts_with('"') && raw.ends_with('"')))
    {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };

    if !inner.contains('\\') {
        return inner.to_string();
    }

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Expand a leading `~/` to the user's home directory.
fn expand_user(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn entry_path(root: &Path) -> PathBuf {
        let dir = root.join("work/2026/07/05");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-07-05T14-30-00-abc123.md");
        fs::write(&path, "body").unwrap();
        path
    }

    fn png_bytes() -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0u8; 16]);
        bytes
    }

    #[test]
    fn cleanup_removes_asset_when_reference_dropped() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("pic.png");
        fs::write(&src, png_bytes()).unwrap();

        let body = format!("![shot]({})", src.display());
        let (new_body, _) = ingest_and_cleanup(&entry, &body, None, true).unwrap();
        let new_body = new_body.unwrap();
        let assets = entry_assets_dir(&entry).unwrap();

        // Re-running with the reference still present keeps the asset.
        let (_, report) = ingest_and_cleanup(&entry, &new_body, None, true).unwrap();
        assert_eq!(report.removed, 0);

        // Dropping the reference removes the asset and prunes the empty dir.
        let (_, report) = ingest_and_cleanup(&entry, "no images", None, true).unwrap();
        assert_eq!(report.removed, 1, "original removed");
        assert!(!assets.exists(), "empty folder pruned");
    }

    #[test]
    fn ingests_local_markdown_image_and_rewrites_ref() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("pic.png");
        fs::write(&src, png_bytes()).unwrap();

        let body = format!("Look:\n![a shot]({})\nend", src.display());
        let (new_body, report) = ingest_and_cleanup(&entry, &body, None, true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        assert_eq!(report.images_stored(), 1);
        assert!(new_body.contains("![a shot](2026-07-05T14-30-00-abc123.assets/"));
        let assets = entry_assets_dir(&entry).unwrap();
        let files: Vec<_> = fs::read_dir(&assets).unwrap().collect();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn wraps_bare_image_path_line() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("bare.png");
        fs::write(&src, png_bytes()).unwrap();

        let body = format!("{}", src.display());
        let (new_body, report) = ingest_and_cleanup(&entry, &body, None, true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        assert_eq!(report.images_stored(), 1);
        assert!(new_body.starts_with("![](2026-07-05T14-30-00-abc123.assets/"));
    }

    #[test]
    fn wraps_bare_image_path_line_with_spaces() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("My Photo.png");
        fs::write(&src, png_bytes()).unwrap();

        let body = format!("{}", src.display());
        let (new_body, report) = ingest_and_cleanup(&entry, &body, None, true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        assert_eq!(report.images_stored(), 1);
        assert!(new_body.starts_with("![](2026-07-05T14-30-00-abc123.assets/"));
    }

    #[test]
    fn wraps_bare_image_path_line_with_escaped_space() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("My Photo.png");
        fs::write(&src, png_bytes()).unwrap();

        // A path dragged/pasted into a terminal escapes the space: `My\ Photo`.
        let body = src.display().to_string().replace(' ', "\\ ");
        assert!(body.contains("\\ "), "test setup should contain an escape");
        let (new_body, report) = ingest_and_cleanup(&entry, &body, None, true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        assert_eq!(report.images_stored(), 1);
        assert!(new_body.starts_with("![](2026-07-05T14-30-00-abc123.assets/"));
    }

    #[test]
    fn unescape_shell_path_handles_escapes_and_quotes() {
        assert_eq!(unescape_shell_path("/a/IMG\\ 2.jpeg"), "/a/IMG 2.jpeg");
        assert_eq!(unescape_shell_path("'/a/My Photo.png'"), "/a/My Photo.png");
        assert_eq!(
            unescape_shell_path("\"/a/My Photo.png\""),
            "/a/My Photo.png"
        );
        assert_eq!(unescape_shell_path("/a/plain.png"), "/a/plain.png");
        assert_eq!(unescape_shell_path("/a/b\\(1\\).png"), "/a/b(1).png");
    }

    #[test]
    fn leaves_prose_with_image_extension_untouched() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());

        // A line ending in an image extension but not a real file is left alone.
        let (changed, report) =
            ingest_and_cleanup(&entry, "here is my summary.png", None, true).unwrap();

        assert!(changed.is_none());
        assert!(report.is_noop());
    }

    #[test]
    fn cleanup_deletes_unreferenced_asset() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let assets = entry_assets_dir(&entry).unwrap();
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("zz.png"), png_bytes()).unwrap();

        // Body references nothing in the folder → the orphan is removed.
        let (changed, report) = ingest_and_cleanup(&entry, "no images here", None, true).unwrap();

        assert!(changed.is_none());
        assert_eq!(report.removed, 1);
        assert!(!assets.exists(), "empty folder should be removed");
    }

    #[test]
    fn keeps_referenced_asset() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let assets = entry_assets_dir(&entry).unwrap();
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("zz.png"), png_bytes()).unwrap();

        let body = "![](2026-07-05T14-30-00-abc123.assets/zz.png)";
        let (changed, report) = ingest_and_cleanup(&entry, body, None, true).unwrap();

        assert!(changed.is_none());
        assert_eq!(report.removed, 0);
        assert!(assets.join("zz.png").exists());
    }

    #[test]
    fn ingests_local_file_attachment_link_and_rewrites_ref() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("report.pdf");
        fs::write(&src, b"%PDF-1.4 data").unwrap();

        let body = format!("See [PDF attachment]({})", src.display());
        let (new_body, report) = ingest_and_cleanup(&entry, &body, None, true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        assert_eq!(report.attachments_stored, 1);
        assert!(
            new_body.starts_with("See [PDF attachment](2026-07-05T14-30-00-abc123.assets/"),
            "link rewritten to canonical: {new_body}"
        );
        assert!(
            new_body.ends_with(".pdf)"),
            "extension preserved: {new_body}"
        );
        let assets = entry_assets_dir(&entry).unwrap();
        assert_eq!(fs::read_dir(&assets).unwrap().count(), 1);
    }

    #[test]
    fn cleanup_keeps_attachment_link_and_removes_unreferenced() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("clip.mp4");
        fs::write(&src, b"video bytes").unwrap();

        let body = format!("[Video attachment]({})", src.display());
        let (stored_body, _) = ingest_and_cleanup(&entry, &body, None, true).unwrap();
        let stored_body = stored_body.unwrap();

        // Re-running with the link present keeps the attachment.
        let (_, report) = ingest_and_cleanup(&entry, &stored_body, None, true).unwrap();
        assert_eq!(report.removed, 0);

        // Dropping the link prunes the attachment.
        let (_, report) = ingest_and_cleanup(&entry, "no links", None, true).unwrap();
        assert_eq!(report.removed, 1);
    }

    #[test]
    fn wraps_bare_non_image_path_line_as_attachment() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("My Report.pdf");
        fs::write(&src, b"%PDF bare").unwrap();

        let body = src.display().to_string();
        let (new_body, report) = ingest_and_cleanup(&entry, &body, None, true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        assert_eq!(report.attachments_stored, 1);
        assert!(
            new_body.starts_with("[My Report.pdf](2026-07-05T14-30-00-abc123.assets/"),
            "labelled by file name: {new_body}"
        );
        assert!(new_body.ends_with(".pdf)"), "{new_body}");
    }

    #[test]
    fn leaves_url_and_anchor_links_untouched() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());

        let body = "[docs](https://example.com/x.pdf) and [top](#heading)";
        let (changed, report) = ingest_and_cleanup(&entry, body, None, true).unwrap();

        assert!(changed.is_none());
        assert!(report.is_noop());
    }

    #[test]
    fn encrypted_attachment_is_written_as_age_and_resolves() {
        let dir = tempdir().unwrap();
        let entry = dir
            .path()
            .join("work/2026/07/05/2026-07-05T14-30-00-abc123.md.age");
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        let paths = KeyPaths::for_config(
            &dir.path().join("config.toml"),
            &dir.path().join("journals"),
        )
        .unwrap();
        crypto::initialize_store_identity(
            &paths,
            "laptop",
            Some(&crate::SecretString::from("secret")),
        )
        .unwrap();

        let src = dir.path().join("notes.pdf");
        fs::write(&src, b"%PDF secret").unwrap();

        let body = format!("[PDF attachment]({})", src.display());
        let (new_body, report) = ingest_and_cleanup(&entry, &body, Some(&paths), true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        assert_eq!(report.attachments_stored, 1);
        // The body link stays clean; only the on-disk file carries `.age`.
        assert!(new_body.contains(".pdf)") && !new_body.contains(".age"));
        let assets = entry_assets_dir(&entry).unwrap();
        let stored = fs::read_dir(&assets)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(stored.to_string_lossy().ends_with(".pdf.age"));

        // The clean link resolves to the encrypted file on disk.
        let file_name = new_body
            .rsplit_once('/')
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .unwrap();
        let resolved = resolve_entry_asset_path(&entry, file_name)
            .unwrap()
            .unwrap();
        assert_eq!(resolved, fs::canonicalize(&stored).unwrap());
        let identity =
            crypto::unlock_identity(&paths, Some(&crate::SecretString::from("secret"))).unwrap();
        let decrypted = crypto::decrypt_file_bytes(&identity, &resolved).unwrap();
        assert_eq!(decrypted.as_bytes(), b"%PDF secret");
    }

    #[test]
    fn report_separates_image_and_attachment_outcomes() {
        let report = AssetReport {
            stored: 3,
            attachments_stored: 1,
            failed: vec![
                AssetFailure::RemoteUnavailable {
                    source: "https://example.com/image.png".to_string(),
                },
                AssetFailure::Ingest {
                    source: "image.png".to_string(),
                    error: "bad image".to_string(),
                },
                AssetFailure::AttachmentIngest {
                    source: "recording.m4a".to_string(),
                    error: "gone".to_string(),
                },
            ],
            ..AssetReport::default()
        };

        assert_eq!(report.images_stored(), 2);
        assert_eq!(report.images_not_stored(), 2);
        assert_eq!(report.attachments_not_stored(), 1);
    }

    #[test]
    fn leaves_internal_ref_untouched() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let assets = entry_assets_dir(&entry).unwrap();
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("zz.png"), png_bytes()).unwrap();

        let body = "![alt](2026-07-05T14-30-00-abc123.assets/zz.png)";
        let (changed, report) = ingest_and_cleanup(&entry, body, None, true).unwrap();

        assert!(changed.is_none());
        assert!(report.is_noop());
    }

    #[test]
    fn duplicate_source_is_stored_once_and_reused() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("pic.png");
        fs::write(&src, png_bytes()).unwrap();

        let body = format!("![one]({})\n![two]({})", src.display(), src.display());
        let (new_body, report) = ingest_and_cleanup(&entry, &body, None, true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        let assets = entry_assets_dir(&entry).unwrap();
        let files: Vec<_> = fs::read_dir(&assets).unwrap().collect();
        assert_eq!(files.len(), 1);
        let links: Vec<_> = new_body.lines().collect();
        assert_eq!(links.len(), 2);
        assert_ne!(links[0], links[1], "alt text differs");
        let first_target = links[0].split('(').nth(1).unwrap();
        let second_target = links[1].split('(').nth(1).unwrap();
        assert_eq!(first_target, second_target);
    }

    #[test]
    fn stored_reference_accepts_only_exact_entry_asset_file() {
        let dir_name = "2026-07-05T14-30-00-abc123.assets";

        let reference = stored_asset_reference(&format!("{dir_name}/x9k2.png"), dir_name)
            .expect("canonical reference should parse");
        assert_eq!(reference.file_name, "x9k2.png");

        assert!(stored_asset_reference("../x9k2.png", dir_name).is_none());
        assert!(stored_asset_reference(&format!("{dir_name}/../x9k2.png"), dir_name).is_none());
        assert!(stored_asset_reference(&format!("{dir_name}/nested/x9k2.png"), dir_name).is_none());
        assert!(stored_asset_reference("/tmp/x9k2.png", dir_name).is_none());
        assert!(stored_asset_reference("https://example.com/x9k2.png", dir_name).is_none());
        assert!(
            stored_asset_reference("2026-07-05T14-30-00-other.assets/x9k2.png", dir_name).is_none()
        );
    }

    #[test]
    fn retarget_stored_links_changes_only_markdown_asset_targets() {
        let source = "old.assets";
        let target = "new.assets";
        let body = concat!(
            "![photo](old.assets/x9k2.png)\n",
            "[recording](old.assets/a1.m4a)\n",
            "ordinary old.assets/x9k2.png text\n",
            "```markdown\n",
            "![example](old.assets/x9k2.png)\n",
            "```\n",
            "![other](different.assets/x9k2.png)\n",
        );

        let rewritten = retarget_stored_asset_links(body, source, target);

        assert_eq!(
            rewritten,
            concat!(
                "![photo](new.assets/x9k2.png)\n",
                "[recording](new.assets/a1.m4a)\n",
                "ordinary old.assets/x9k2.png text\n",
                "```markdown\n",
                "![example](old.assets/x9k2.png)\n",
                "```\n",
                "![other](different.assets/x9k2.png)\n",
            )
        );
    }

    #[test]
    fn sole_stored_image_matches_in_folder_line() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());

        let (alt, file) = sole_stored_image(
            "![a shot](2026-07-05T14-30-00-abc123.assets/x9k2.png)",
            &entry,
        )
        .expect("should match");
        assert_eq!(alt, "a shot");
        assert_eq!(file, "x9k2.png");

        // Leading/trailing whitespace around the sole image is ignored.
        assert!(
            sole_stored_image(
                "   ![](2026-07-05T14-30-00-abc123.assets/x9k2.png)  ",
                &entry
            )
            .is_some()
        );
    }

    #[test]
    fn sole_stored_image_rejects_external_wrong_folder_and_traversal() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());

        assert!(sole_stored_image("![](https://example.com/x.png)", &entry).is_none());
        assert!(sole_stored_image("![](/etc/x.png)", &entry).is_none());
        assert!(sole_stored_image("![](other/x9k2.png)", &entry).is_none());
        assert!(
            sole_stored_image("![](2026-07-05T14-30-00-other.assets/x9k2.png)", &entry).is_none()
        );
        assert!(
            sole_stored_image("![](2026-07-05T14-30-00-abc123.assets/../x9k2.png)", &entry)
                .is_none()
        );
    }

    #[test]
    fn sole_stored_image_rejects_extra_text_or_multiple_images() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let asset = "2026-07-05T14-30-00-abc123.assets/x9k2.png";

        assert!(sole_stored_image(&format!("look ![]({asset})"), &entry).is_none());
        assert!(sole_stored_image(&format!("![]({asset}) trailing"), &entry).is_none());
        assert!(sole_stored_image(&format!("![]({asset}) ![]({asset})"), &entry).is_none());
    }

    #[test]
    fn resolve_entry_asset_path_rejects_traversal_and_wrong_folder() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let assets = entry_assets_dir(&entry).unwrap();
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("x9k2.png"), png_bytes()).unwrap();

        assert!(
            resolve_entry_asset_path(&entry, "x9k2.png")
                .unwrap()
                .is_some()
        );
        assert!(
            resolve_entry_asset_path(&entry, "../x9k2.png")
                .unwrap()
                .is_none()
        );
        assert!(
            resolve_entry_asset_path(&entry, "nested/x9k2.png")
                .unwrap()
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_entry_asset_path_rejects_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let assets = entry_assets_dir(&entry).unwrap();
        fs::create_dir_all(&assets).unwrap();
        let outside = dir.path().join("outside.png");
        fs::write(&outside, png_bytes()).unwrap();
        symlink(&outside, assets.join("linked.png")).unwrap();

        assert!(
            resolve_entry_asset_path(&entry, "linked.png")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn skips_remote_when_downloads_disabled() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());

        let body = "![](https://example.com/pic.png)";
        let (changed, report) = ingest_and_cleanup(&entry, body, None, false).unwrap();

        assert!(changed.is_none());
        assert_eq!(report.stored, 0);
        assert_eq!(
            report.failed,
            vec![AssetFailure::RemoteUnavailable {
                source: "https://example.com/pic.png".to_string(),
            }]
        );
    }

    #[test]
    fn encrypted_asset_is_written_as_age_and_round_trips() {
        let dir = tempdir().unwrap();
        let entry = dir
            .path()
            .join("work/2026/07/05/2026-07-05T14-30-00-abc123.md.age");
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        let paths = KeyPaths::for_config(
            &dir.path().join("config.toml"),
            &dir.path().join("journals"),
        )
        .unwrap();
        crypto::initialize_store_identity(
            &paths,
            "laptop",
            Some(&crate::SecretString::from("secret")),
        )
        .unwrap();
        let identity =
            crypto::unlock_identity(&paths, Some(&crate::SecretString::from("secret"))).unwrap();

        let src = dir.path().join("pic.png");
        let original = png_bytes();
        fs::write(&src, &original).unwrap();

        let body = format!("![shot]({})", src.display());
        let (new_body, report) = ingest_and_cleanup(&entry, &body, Some(&paths), true).unwrap();

        let new_body = new_body.expect("body should change");
        assert_eq!(report.stored, 1);
        // The body link stays clean (no `.age`) even though the store is encrypted;
        // only the file on disk carries the `.age` suffix.
        assert!(
            new_body.contains(".png)") && !new_body.contains(".age"),
            "link should stay clean: {new_body}"
        );
        let assets = entry_assets_dir(&entry).unwrap();
        let stored = fs::read_dir(&assets)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(stored.to_string_lossy().ends_with(".png.age"));
        let decrypted = crypto::decrypt_file_bytes(&identity, &stored).unwrap();
        assert_eq!(decrypted.as_bytes(), original);

        // The clean link resolves to the encrypted file on disk.
        let file_name = new_body
            .rsplit_once('/')
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .unwrap();
        let resolved = resolve_entry_asset_path(&entry, file_name)
            .unwrap()
            .unwrap();
        assert_eq!(resolved, fs::canonicalize(&stored).unwrap());
    }

    #[test]
    fn ignores_image_inside_code_fence() {
        let dir = tempdir().unwrap();
        let entry = entry_path(dir.path());
        let src = dir.path().join("pic.png");
        fs::write(&src, png_bytes()).unwrap();

        let body = format!("```\n![x]({})\n```", src.display());
        let (changed, report) = ingest_and_cleanup(&entry, &body, None, true).unwrap();

        assert!(changed.is_none());
        assert!(report.is_noop());
    }
}
