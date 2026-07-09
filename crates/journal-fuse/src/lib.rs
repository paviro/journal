//! A FUSE filesystem that exposes an encrypted journal as an ordinary, decrypted
//! directory tree, re-encrypting everything written back. The store stays
//! encrypted on disk; plaintext only ever lives in this process's memory.
//!
//! The mount mirrors the on-disk layout with the encryption suffix stripped from
//! file names: an entry stored as `…/2026/07/09/<id>.md.age` appears as
//! `…/2026/07/09/<id>.md`, and an encrypted asset `<id>.assets/photo.jpg.age`
//! appears as `<id>.assets/photo.jpg`. Every file — entry or asset — is handled
//! the same way: decrypt on read, re-encrypt on write, keyed purely off whether
//! the on-disk name ends in `.age`.
//!
//! The mount is fully read-write: reading, editing, creating, deleting, and
//! renaming files and directories all work and are reflected on disk (encrypted
//! when the store is encrypted). Because age is a whole-file format, writes are
//! buffered per open handle and the whole file is re-encrypted on flush/release
//! rather than encrypted piecemeal.
//!
//! Implementation: this drives libfuse's own high-level (path-based) event loop
//! through a small C bridge (`bridge.c`), rather than reimplementing the FUSE
//! kernel protocol. That is what lets it work with kext-free backends such as
//! fuse-t on macOS, and it keeps one backend for Linux and macOS alike. Because
//! the API is path-based, there is no inode bookkeeping — each path maps straight
//! to its on-disk file.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use journal_core::AppResult;
use journal_storage::{JournalStore, StoreFileEncoding};
use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

unsafe extern "C" {
    fn journal_fuse_run(
        mountpoint: *const c_char,
        context: *mut c_void,
        volname: *const c_char,
    ) -> c_int;
    fn bridge_fill(filler: *mut c_void, buf: *mut c_void, name: *const c_char) -> c_int;
}

/// An open file: the whole file buffered as plaintext bytes, its on-disk path
/// (with `.age` when encrypted), and whether it was opened writable / is dirty.
struct Handle {
    on_disk: PathBuf,
    encoding: StoreFileEncoding,
    buf: Vec<u8>,
    dirty: bool,
    writable: bool,
}

/// Per-mount state, reached from every callback via libfuse's `private_data`.
/// A single dispatch thread (`-s`) serializes access, but the mutex keeps it
/// sound and simple.
struct Ctx {
    store: JournalStore,
    root: PathBuf,
    inner: Mutex<Inner>,
    uid: u32,
    gid: u32,
}

struct Inner {
    handles: HashMap<u64, Handle>,
    next_fh: u64,
    /// Plaintext byte length per on-disk file, so `getattr` need not decrypt every
    /// time. Dropped whenever the file is written.
    sizes: HashMap<PathBuf, u64>,
}

/// Mount `store` (already unlocked) at `mountpoint`, exposing its entries and
/// assets as a decrypted, writable tree. Blocks until the filesystem is
/// unmounted. The caller must unlock the store's identity first.
pub fn mount(store: JournalStore, mountpoint: &Path) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    ensure_macfuse_loaded();

    let root = store.paths().journal_root.clone();
    let ctx = Box::new(Ctx {
        store,
        root,
        inner: Mutex::new(Inner {
            handles: HashMap::new(),
            next_fh: 1,
            sizes: HashMap::new(),
        }),
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
    });
    let ctx_ptr = Box::into_raw(ctx) as *mut c_void;

    let mountpoint = CString::new(mountpoint.as_os_str().as_bytes())?;
    // macFUSE/fuse-t show a Finder volume regardless of the mount path; name it.
    let volname = if cfg!(target_os = "macos") {
        Some(CString::new("Journals").unwrap())
    } else {
        None
    };
    let volname_ptr = volname.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());

    let _status = unsafe { journal_fuse_run(mountpoint.as_ptr(), ctx_ptr, volname_ptr) };

    // Reclaim the leaked context now that the event loop has returned.
    drop(unsafe { Box::from_raw(ctx_ptr as *mut Ctx) });

    // A nonzero status here is not an application error: an external unmount
    // reports one (fuse-t exits 8), and libfuse prints its own message for real
    // setup failures. Either way the session is over, so return cleanly.
    Ok(())
}

/// macOS only: macFUSE does not always auto-load its kernel extension, and without
/// it there is no `/dev/macfuse` device to mount onto. When the setuid-root loader
/// helper is present and no device exists yet, run it. Harmless (a no-op) with
/// fuse-t, which needs no kext.
#[cfg(target_os = "macos")]
fn ensure_macfuse_loaded() {
    if std::path::Path::new("/dev/macfuse0").exists() {
        return;
    }
    let loader = "/Library/Filesystems/macfuse.fs/Contents/Resources/load_macfuse";
    if std::path::Path::new(loader).exists() {
        let _ = std::process::Command::new(loader).status();
    }
}

/// `rename` flags as delivered by the FUSE protocol (kernel values, the same
/// across libfuse/macFUSE/fuse-t). `libc`'s `RENAME_*` constants differ per
/// platform, so we pin the wire values here.
const RENAME_NOREPLACE: u32 = 1;
const RENAME_EXCHANGE: u32 = 2;

// --- path helpers -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct BackingFile {
    path: PathBuf,
    encoding: StoreFileEncoding,
}

fn ctx_ref<'a>(ptr: *mut c_void) -> &'a Ctx {
    unsafe { &*(ptr as *const Ctx) }
}

/// The on-disk *base* path for a mounted path (encryption suffix not yet applied):
/// e.g. `/personal/x.md` → `<root>/personal/x.md`. The FUSE root `/` maps to the
/// journal root itself.
fn base_of(ctx: &Ctx, path: *const c_char) -> PathBuf {
    let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
    let rel = bytes.strip_prefix(b"/").unwrap_or(bytes);
    if rel.is_empty() {
        ctx.root.clone()
    } else {
        ctx.root.join(OsStr::from_bytes(rel))
    }
}

/// Append the age extension: `x.md` → `x.md.age`.
fn with_age(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".age");
    PathBuf::from(name)
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| {
            let ty = meta.file_type();
            ty.is_file() && !ty.is_symlink()
        })
        .unwrap_or(false)
}

fn is_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| {
            let ty = meta.file_type();
            ty.is_dir() && !ty.is_symlink()
        })
        .unwrap_or(false)
}

/// Resolve a mounted file's actual on-disk path: the encrypted `<base>.age` if
/// it exists, else the plain `<base>`, else `None` when neither exists.
fn existing_file(base: &Path) -> Option<BackingFile> {
    let encrypted = with_age(base);
    if is_regular_file(&encrypted) {
        Some(BackingFile {
            path: encrypted,
            encoding: StoreFileEncoding::Encrypted,
        })
    } else if is_regular_file(base) {
        Some(BackingFile {
            path: base.to_path_buf(),
            encoding: StoreFileEncoding::Plain,
        })
    } else {
        None
    }
}

fn strip_age_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let mounted_name = name.strip_suffix(".age")?;
    Some(path.with_file_name(mounted_name))
}

fn mounted_name_for_backing(path: &Path, name: &OsStr) -> OsString {
    if let Some(base) = strip_age_path(path)
        && should_encrypt_new_file(&base)
    {
        return mounted_name(name);
    }
    name.to_os_string()
}

fn should_encrypt_new_file(base: &Path) -> bool {
    let Some(name) = base.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.ends_with(".md") || name.contains(".md.") {
        return true;
    }
    base.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| name.ends_with(".assets"))
    })
}

fn backing_for_new_file(base: PathBuf) -> BackingFile {
    if should_encrypt_new_file(&base) {
        BackingFile {
            path: with_age(&base),
            encoding: StoreFileEncoding::Encrypted,
        }
    } else {
        BackingFile {
            path: base,
            encoding: StoreFileEncoding::Plain,
        }
    }
}

fn is_safe_mounted_path(path: *const c_char) -> bool {
    let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
    let rel = bytes.strip_prefix(b"/").unwrap_or(bytes);
    rel.split(|&b| b == b'/')
        .all(|component| component.is_empty() || (component != b"." && component != b".."))
}

fn component_is_rejected_system_state(component: &[u8]) -> bool {
    component.starts_with(b"._")
        || matches!(
            std::str::from_utf8(component).ok(),
            Some(
                ".Spotlight-V100"
                    | ".fseventsd"
                    | ".Trashes"
                    | ".TemporaryItems"
                    | ".DocumentRevisions-V100"
                    | ".apdisk"
            )
        )
}

fn is_rejected_system_path(path: *const c_char) -> bool {
    let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
    let rel = bytes.strip_prefix(b"/").unwrap_or(bytes);
    rel.split(|&b| b == b'/')
        .any(component_is_rejected_system_state)
}

fn is_rejected_system_name(name: &OsStr) -> bool {
    component_is_rejected_system_state(name.as_bytes())
}

/// Whether a mounted path lies inside the `.age` encryption metadata directory
/// (the recipients roster and pending join requests). The mount refuses to reveal
/// or touch it: a stray tool writing there could corrupt the roster and lock the
/// store. `.trash`, by contrast, holds ordinary encrypted entries and stays
/// browsable so they can be recovered.
fn is_encryption_metadata(path: *const c_char) -> bool {
    let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
    let rel = bytes.strip_prefix(b"/").unwrap_or(bytes);
    rel.split(|&b| b == b'/').next() == Some(b".age")
}

fn is_protected_path(path: *const c_char) -> bool {
    !is_safe_mounted_path(path) || is_encryption_metadata(path) || is_rejected_system_path(path)
}

/// Names to show for a mounted directory: the `.age` encryption folder and
/// rejected system state are hidden, `.trash` is kept so deleted entries can be
/// recovered, and the `.age` suffix is stripped only from encrypted backing
/// files.
fn visible_entries(base: &Path) -> Vec<OsString> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let disk_name = entry.file_name();
        let bytes = disk_name.as_bytes();
        if bytes == b".age" || is_rejected_system_name(&disk_name) {
            continue;
        }
        if disk_name
            .to_str()
            .is_some_and(|name| name.ends_with(".age"))
        {
            names.push(mounted_name_for_backing(&entry.path(), &disk_name));
        } else if existing_file(&entry.path())
            .is_some_and(|file| file.encoding == StoreFileEncoding::Plain)
        {
            names.push(disk_name);
        } else if is_directory(&entry.path()) {
            names.push(disk_name);
        }
    }
    names
}

/// Map an on-disk basename to how it appears in the mount: `x.md.age` → `x.md`.
fn mounted_name(disk_name: &OsStr) -> OsString {
    match disk_name.to_str() {
        Some(s) => OsString::from(s.strip_suffix(".age").unwrap_or(s)),
        None => disk_name.to_os_string(),
    }
}

impl Ctx {
    /// Lock the mutable state, recovering from poisoning: a panic caught at the
    /// FFI boundary (see [`guard`]) can poison the lock, but the protected data
    /// stays structurally valid, so we keep serving the mount rather than
    /// wedging every later call into a panic.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Current plaintext length of an on-disk file, preferring an open handle's
    /// live buffer (so `stat` stays consistent mid-edit) and otherwise decrypting
    /// once and caching. Propagates a read/decrypt failure rather than reporting a
    /// non-empty file as zero-length: the store is unlocked at mount time, so a
    /// failure here is real corruption, not an empty file.
    fn file_size(&self, file: &BackingFile) -> AppResult<u64> {
        let inner = self.lock();
        if let Some(handle) = inner.handles.values().find(|h| h.on_disk == file.path) {
            return Ok(handle.buf.len() as u64);
        }
        if let Some(&size) = inner.sizes.get(&file.path) {
            return Ok(size);
        }
        drop(inner);
        let size = self.store.read_store_file(&file.path, file.encoding)?.len() as u64;
        self.inner
            .lock()
            .unwrap()
            .sizes
            .insert(file.path.clone(), size);
        Ok(size)
    }

    /// Re-encrypt and write back a dirty handle's buffer; no-op for clean or
    /// read-only handles.
    fn commit(&self, fh: u64) -> Result<(), c_int> {
        let inner = self.lock();
        let Some(handle) = inner.handles.get(&fh) else {
            return Ok(());
        };
        if !handle.dirty || !handle.writable {
            return Ok(());
        }
        let file = BackingFile {
            path: handle.on_disk.clone(),
            encoding: handle.encoding,
        };
        let bytes = handle.buf.clone();
        drop(inner);
        self.store
            .write_store_file(&file.path, file.encoding, &bytes)
            .map_err(|e| app_errno(&e))?;
        let mut inner = self.lock();
        if let Some(handle) = inner.handles.get_mut(&fh) {
            handle.dirty = false;
        }
        inner.sizes.remove(&file.path);
        Ok(())
    }
}

fn errno(err: &std::io::Error) -> c_int {
    -err.raw_os_error().unwrap_or(libc::EIO)
}

/// Run a callback body, turning any panic into `-EIO` instead of letting it
/// unwind across the C boundary (which aborts the whole mount, losing every
/// other buffered handle). The default panic hook still logs the message first.
fn guard(f: impl FnOnce() -> c_int) -> c_int {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(-libc::EIO)
}

/// Best-effort OS errno for a store error, so failures like a full disk
/// (`ENOSPC`) or a permission problem (`EACCES`) reach the caller instead of a
/// blanket `EIO`. Walks the error chain for the first `io::Error` carrying a raw
/// errno; falls back to `EIO` (e.g. a decrypt failure, which has no OS code).
fn app_errno(err: &anyhow::Error) -> c_int {
    for cause in err.chain() {
        if let Some(code) = cause
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::raw_os_error)
        {
            return -code;
        }
    }
    -libc::EIO
}

fn fill_dir_stat(ctx: &Ctx, st: *mut libc::stat, disk: &Path) {
    unsafe {
        let s = &mut *st;
        s.st_mode = libc::S_IFDIR | 0o755;
        s.st_nlink = 2;
        s.st_uid = ctx.uid;
        s.st_gid = ctx.gid;
    }
    set_mtime_from_disk(st, disk);
}

fn fill_file_stat(ctx: &Ctx, st: *mut libc::stat, size: u64, disk: &Path) {
    unsafe {
        let s = &mut *st;
        s.st_mode = libc::S_IFREG | 0o644;
        s.st_nlink = 1;
        s.st_uid = ctx.uid;
        s.st_gid = ctx.gid;
        s.st_size = size as libc::off_t;
        s.st_blocks = size.div_ceil(512) as libc::blkcnt_t;
    }
    set_mtime_from_disk(st, disk);
}

/// Copy the on-disk modification time onto the stat, so files and folders show
/// their real dates (without this, directories report the epoch — 1970).
fn set_mtime_from_disk(st: *mut libc::stat, disk: &Path) {
    if let Ok(secs) = std::fs::metadata(disk)
        .and_then(|m| m.modified())
        .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
        .map(|d| d.as_secs() as libc::time_t)
    {
        set_mtime(st, secs);
    }
}

fn set_mtime(st: *mut libc::stat, secs: libc::time_t) {
    unsafe {
        let s = &mut *st;
        s.st_mtime = secs;
        s.st_atime = secs;
        s.st_ctime = secs;
    }
}

// --- libfuse callbacks (called from bridge.c) -------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn jf_getattr(ctx: *mut c_void, path: *const c_char, st: *mut libc::stat) -> c_int {
    guard(move || getattr(ctx, path, st))
}

fn getattr(ctx: *mut c_void, path: *const c_char, st: *mut libc::stat) -> c_int {
    let ctx = ctx_ref(ctx);
    unsafe { std::ptr::write_bytes(st, 0, 1) };
    if is_protected_path(path) {
        return -libc::ENOENT;
    }
    let base = base_of(ctx, path);
    if is_directory(&base) {
        fill_dir_stat(ctx, st, &base);
        0
    } else if let Some(file) = existing_file(&base) {
        match ctx.file_size(&file) {
            Ok(size) => {
                fill_file_stat(ctx, st, size, &file.path);
                0
            }
            Err(e) => app_errno(&e),
        }
    } else {
        -libc::ENOENT
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_readdir(
    ctx: *mut c_void,
    path: *const c_char,
    buf: *mut c_void,
    filler: *mut c_void,
) -> c_int {
    guard(move || readdir(ctx, path, buf, filler))
}

fn readdir(ctx: *mut c_void, path: *const c_char, buf: *mut c_void, filler: *mut c_void) -> c_int {
    let ctx = ctx_ref(ctx);
    if is_protected_path(path) {
        return -libc::ENOENT;
    }
    for name in [c".", c".."] {
        unsafe { bridge_fill(filler, buf, name.as_ptr()) };
    }
    let base = base_of(ctx, path);
    for name in visible_entries(&base) {
        if let Ok(name) = CString::new(name.as_bytes()) {
            unsafe { bridge_fill(filler, buf, name.as_ptr()) };
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_open(
    ctx: *mut c_void,
    path: *const c_char,
    flags: c_int,
    fh_out: *mut u64,
) -> c_int {
    guard(move || open(ctx, path, flags, fh_out))
}

fn open(ctx: *mut c_void, path: *const c_char, flags: c_int, fh_out: *mut u64) -> c_int {
    let ctx = ctx_ref(ctx);
    if is_protected_path(path) {
        return -libc::ENOENT;
    }
    let base = base_of(ctx, path);
    let Some(file) = existing_file(&base) else {
        return -libc::ENOENT;
    };
    let access = flags & libc::O_ACCMODE;
    let writable = access == libc::O_WRONLY || access == libc::O_RDWR;
    let truncate = flags & libc::O_TRUNC != 0;
    let buf = if truncate {
        Vec::new()
    } else {
        match ctx.store.read_store_file(&file.path, file.encoding) {
            Ok(bytes) => bytes,
            Err(e) => return app_errno(&e),
        }
    };
    let mut inner = ctx.lock();
    let fh = inner.next_fh;
    inner.next_fh += 1;
    inner.handles.insert(
        fh,
        Handle {
            on_disk: file.path,
            encoding: file.encoding,
            buf,
            dirty: truncate && writable,
            writable,
        },
    );
    unsafe { *fh_out = fh };
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_create(
    ctx: *mut c_void,
    path: *const c_char,
    mode: u32,
    flags: c_int,
    fh_out: *mut u64,
) -> c_int {
    guard(move || create(ctx, path, mode, flags, fh_out))
}

fn create(
    ctx: *mut c_void,
    path: *const c_char,
    _mode: u32,
    flags: c_int,
    fh_out: *mut u64,
) -> c_int {
    let ctx = ctx_ref(ctx);
    if is_protected_path(path) {
        return -libc::ENOENT;
    }
    let base = base_of(ctx, path);
    if existing_file(&base).is_some() {
        return -libc::EEXIST;
    }
    let file = backing_for_new_file(base);
    if let Err(e) = ctx.store.write_store_file(&file.path, file.encoding, &[]) {
        return app_errno(&e);
    }
    let access = flags & libc::O_ACCMODE;
    let writable = access == libc::O_WRONLY || access == libc::O_RDWR;
    let mut inner = ctx.lock();
    let fh = inner.next_fh;
    inner.next_fh += 1;
    inner.handles.insert(
        fh,
        Handle {
            on_disk: file.path,
            encoding: file.encoding,
            buf: Vec::new(),
            dirty: false,
            writable,
        },
    );
    unsafe { *fh_out = fh };
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_read(
    ctx: *mut c_void,
    path: *const c_char,
    buf: *mut c_char,
    size: usize,
    off: i64,
    fh: u64,
) -> c_int {
    guard(move || read(ctx, path, buf, size, off, fh))
}

fn read(
    ctx: *mut c_void,
    _path: *const c_char,
    buf: *mut c_char,
    size: usize,
    off: i64,
    fh: u64,
) -> c_int {
    let ctx = ctx_ref(ctx);
    let inner = ctx.lock();
    let Some(handle) = inner.handles.get(&fh) else {
        return -libc::EBADF;
    };
    let start = (off.max(0) as usize).min(handle.buf.len());
    let end = start.saturating_add(size).min(handle.buf.len());
    let slice = &handle.buf[start..end];
    unsafe { std::ptr::copy_nonoverlapping(slice.as_ptr(), buf as *mut u8, slice.len()) };
    slice.len() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_write(
    ctx: *mut c_void,
    path: *const c_char,
    buf: *const c_char,
    size: usize,
    off: i64,
    fh: u64,
) -> c_int {
    guard(move || write(ctx, path, buf, size, off, fh))
}

fn write(
    ctx: *mut c_void,
    _path: *const c_char,
    buf: *const c_char,
    size: usize,
    off: i64,
    fh: u64,
) -> c_int {
    let ctx = ctx_ref(ctx);
    let mut inner = ctx.lock();
    let Some(handle) = inner.handles.get_mut(&fh) else {
        return -libc::EBADF;
    };
    if !handle.writable {
        return -libc::EBADF;
    }
    let start = off.max(0) as usize;
    let Some(end) = start.checked_add(size) else {
        return -libc::EFBIG;
    };
    if handle.buf.len() < end {
        handle.buf.resize(end, 0);
    }
    let data = unsafe { std::slice::from_raw_parts(buf as *const u8, size) };
    handle.buf[start..end].copy_from_slice(data);
    handle.dirty = true;
    size as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_truncate(
    ctx: *mut c_void,
    path: *const c_char,
    size: i64,
    fh: u64,
    has_fh: c_int,
) -> c_int {
    guard(move || truncate(ctx, path, size, fh, has_fh))
}

fn truncate(ctx: *mut c_void, path: *const c_char, size: i64, _fh: u64, _has_fh: c_int) -> c_int {
    let ctx = ctx_ref(ctx);
    if is_protected_path(path) {
        return -libc::ENOENT;
    }
    let size = size.max(0) as usize;
    let base = base_of(ctx, path);
    let file = existing_file(&base);

    // Resize every open handle for this file so a later `release` commits the
    // truncated length. This matters because backends like fuse-t (NFS) deliver
    // `>`-style truncation as a standalone SETATTR, decoupled from the write's
    // open handle — without this, a shorter rewrite would leave the old tail.
    let mut inner = ctx.lock();
    let mut touched = false;
    if let Some(file) = &file {
        for handle in inner
            .handles
            .values_mut()
            .filter(|handle| handle.on_disk == file.path)
        {
            handle.buf.resize(size, 0);
            handle.dirty = true;
            touched = true;
        }
    }
    if touched {
        return 0;
    }
    drop(inner);

    // No open handle: rewrite the file on disk directly.
    let Some(file) = file else {
        return -libc::ENOENT;
    };
    let mut bytes = match ctx.store.read_store_file(&file.path, file.encoding) {
        Ok(bytes) => bytes,
        Err(e) => return app_errno(&e),
    };
    bytes.resize(size, 0);
    if let Err(e) = ctx
        .store
        .write_store_file(&file.path, file.encoding, &bytes)
    {
        return app_errno(&e);
    }
    ctx.lock().sizes.remove(&file.path);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_unlink(ctx: *mut c_void, path: *const c_char) -> c_int {
    guard(move || unlink(ctx, path))
}

fn unlink(ctx: *mut c_void, path: *const c_char) -> c_int {
    let ctx = ctx_ref(ctx);
    if is_protected_path(path) {
        return -libc::ENOENT;
    }
    let base = base_of(ctx, path);
    let Some(file) = existing_file(&base) else {
        return -libc::ENOENT;
    };
    match std::fs::remove_file(&file.path) {
        Ok(()) => {
            ctx.lock().sizes.remove(&file.path);
            0
        }
        Err(err) => errno(&err),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_mkdir(ctx: *mut c_void, path: *const c_char, mode: u32) -> c_int {
    guard(move || mkdir(ctx, path, mode))
}

fn mkdir(ctx: *mut c_void, path: *const c_char, _mode: u32) -> c_int {
    let ctx = ctx_ref(ctx);
    if is_protected_path(path) {
        return -libc::ENOENT;
    }
    match std::fs::create_dir(base_of(ctx, path)) {
        Ok(()) => 0,
        Err(err) => errno(&err),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_rmdir(ctx: *mut c_void, path: *const c_char) -> c_int {
    guard(move || rmdir(ctx, path))
}

fn rmdir(ctx: *mut c_void, path: *const c_char) -> c_int {
    let ctx = ctx_ref(ctx);
    if is_protected_path(path) {
        return -libc::ENOENT;
    }
    let dir = base_of(ctx, path);
    if let Err(err) = std::fs::remove_dir(&dir) {
        // A folder the mount shows as empty can still hold hidden OS junk on disk
        // (a stray .DS_Store, AppleDouble `._*`) that the user can't see to
        // delete. Clear it and retry so the delete they asked for goes through.
        if err.raw_os_error() == Some(libc::ENOTEMPTY) && visible_entries(&dir).is_empty() {
            remove_rejected_system_state_in(&dir);
            return match std::fs::remove_dir(&dir) {
                Ok(()) => 0,
                Err(err) => errno(&err),
            };
        }
        return errno(&err);
    }
    0
}

/// Delete rejected system state directly inside `dir`. Used only when the mount
/// already treats `dir` as empty, so nothing the user can see is lost.
fn remove_rejected_system_state_in(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if is_rejected_system_name(&entry.file_name()) {
            let path = entry.path();
            let _ = std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir_all(&path));
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_rename(
    ctx: *mut c_void,
    from: *const c_char,
    to: *const c_char,
    flags: u32,
) -> c_int {
    guard(move || rename(ctx, from, to, flags))
}

fn rename(ctx: *mut c_void, from: *const c_char, to: *const c_char, flags: u32) -> c_int {
    let ctx = ctx_ref(ctx);
    if is_protected_path(from) || is_protected_path(to) {
        return -libc::ENOENT;
    }
    // Atomic swap isn't supported (the two sides may differ in encryption).
    if flags & RENAME_EXCHANGE != 0 {
        return -libc::ENOSYS;
    }
    let from_base = base_of(ctx, from);
    let to_base = base_of(ctx, to);
    // Honor RENAME_NOREPLACE: never clobber an existing destination when the
    // caller asked us not to. std::fs::rename overwrites unconditionally.
    if flags & RENAME_NOREPLACE != 0
        && (is_directory(&to_base) || existing_file(&to_base).is_some())
    {
        return -libc::EEXIST;
    }
    // Resolve the source: a directory keeps its plain name; a file keeps its
    // backing encoding across the move.
    let (from_disk, to_disk) = if is_directory(&from_base) {
        (from_base, to_base)
    } else if let Some(src) = existing_file(&from_base) {
        let dst = match src.encoding {
            StoreFileEncoding::Encrypted => with_age(&to_base),
            StoreFileEncoding::Plain => to_base,
        };
        (src.path, dst)
    } else {
        return -libc::ENOENT;
    };
    match std::fs::rename(&from_disk, &to_disk) {
        Ok(()) => {
            let mut inner = ctx.lock();
            inner.sizes.remove(&from_disk);
            inner.sizes.remove(&to_disk);
            0
        }
        Err(err) => errno(&err),
    }
}

/// Report the backing filesystem's space so `df` shows real numbers and GUI file
/// managers (Finder) allow copies in — they refuse without a free-space figure.
#[unsafe(no_mangle)]
pub extern "C" fn jf_statfs(
    ctx: *mut c_void,
    path: *const c_char,
    st: *mut libc::statvfs,
) -> c_int {
    guard(move || statfs(ctx, path, st))
}

fn statfs(ctx: *mut c_void, _path: *const c_char, st: *mut libc::statvfs) -> c_int {
    let ctx = ctx_ref(ctx);
    let Ok(root) = CString::new(ctx.root.as_os_str().as_bytes()) else {
        return -libc::EIO;
    };
    let rc = unsafe { libc::statvfs(root.as_ptr(), st) };
    if rc == 0 {
        0
    } else {
        errno(&std::io::Error::last_os_error())
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_flush(ctx: *mut c_void, path: *const c_char, fh: u64) -> c_int {
    guard(move || flush(ctx, path, fh))
}

fn flush(ctx: *mut c_void, _path: *const c_char, fh: u64) -> c_int {
    match ctx_ref(ctx).commit(fh) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jf_release(ctx: *mut c_void, path: *const c_char, fh: u64) -> c_int {
    guard(move || release(ctx, path, fh))
}

fn release(ctx: *mut c_void, _path: *const c_char, fh: u64) -> c_int {
    let ctx = ctx_ref(ctx);
    let result = ctx.commit(fh);
    ctx.lock().handles.remove(&fh);
    match result {
        Ok(()) => 0,
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use journal_storage::SecretString;

    #[test]
    fn with_age_and_mounted_name_round_trip() {
        let base = Path::new("/j/2026/x.md");
        assert_eq!(with_age(base), Path::new("/j/2026/x.md.age"));
        assert_eq!(mounted_name(OsStr::new("x.md.age")), OsString::from("x.md"));
        assert_eq!(
            mounted_name(OsStr::new("photo.jpg.age")),
            OsString::from("photo.jpg")
        );
        assert_eq!(
            mounted_name(OsStr::new("plain.md")),
            OsString::from("plain.md")
        );
    }

    #[test]
    fn rejected_system_state_excludes_benign_metadata_and_editor_tempfiles() {
        for rejected in [
            "._entry.md",
            ".Trashes",
            ".Spotlight-V100",
            ".fseventsd",
            ".TemporaryItems",
            ".DocumentRevisions-V100",
            ".apdisk",
        ] {
            assert!(
                is_rejected_system_name(OsStr::new(rejected)),
                "{rejected} should be rejected"
            );
        }
        for ok in [
            ".DS_Store",
            ".metadata_never_index",
            ".VolumeIcon.icns",
            "desktop.ini",
            "Thumbs.db",
            ".entry.md.swp",
            ".#entry.md",
            "entry.md",
            "photo.jpg",
        ] {
            assert!(
                !is_rejected_system_name(OsStr::new(ok)),
                "{ok} should be allowed"
            );
        }
    }

    #[test]
    fn existing_file_prefers_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("entry.md");
        assert_eq!(existing_file(&base), None);
        std::fs::write(with_age(&base), b"ct").unwrap();
        assert_eq!(
            existing_file(&base),
            Some(BackingFile {
                path: with_age(&base),
                encoding: StoreFileEncoding::Encrypted,
            })
        );
    }

    // --- callback-level virtual filesystem tests ----------------------------
    //
    // The jf_* callbacks are driven directly against an unlocked encrypted store
    // in a tempdir, so the whole read/write/encryption path is exercised with no
    // libfuse runtime.

    struct Fixture {
        _dir: tempfile::TempDir,
        ctx: *mut c_void,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut store = JournalStore::new(dir.path().join("journals"), dir.path());
            store.ensure().unwrap();
            store
                .initialize_encryption("test", Some(&SecretString::from("pw")))
                .unwrap();
            store.unlock(Some(&SecretString::from("pw"))).unwrap();
            assert!(store.encrypts_new_files());
            let root = store.paths().journal_root.clone();
            let ctx = Box::new(Ctx {
                store,
                root,
                inner: Mutex::new(Inner {
                    handles: HashMap::new(),
                    next_fh: 1,
                    sizes: HashMap::new(),
                }),
                uid: 0,
                gid: 0,
            });
            Self {
                _dir: dir,
                ctx: Box::into_raw(ctx) as *mut c_void,
            }
        }

        fn root(&self) -> PathBuf {
            ctx_ref(self.ctx).root.clone()
        }

        /// Decrypt an on-disk store file back to plaintext.
        fn read_disk(&self, disk: &Path) -> Vec<u8> {
            ctx_ref(self.ctx)
                .store
                .read_store_file(disk, StoreFileEncoding::Encrypted)
                .unwrap()
        }

        fn mkdir_p(&self, rel: &str) {
            std::fs::create_dir_all(self.root().join(rel)).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            drop(unsafe { Box::from_raw(self.ctx as *mut Ctx) });
        }
    }

    fn cpath(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// create → write-at-0 → release a new file.
    fn write_new(fx: &Fixture, path: &str, data: &[u8]) {
        let p = cpath(path);
        let mut fh = 0u64;
        assert_eq!(
            jf_create(fx.ctx, p.as_ptr(), 0o644, libc::O_WRONLY, &mut fh),
            0
        );
        assert_eq!(
            jf_write(
                fx.ctx,
                p.as_ptr(),
                data.as_ptr() as *const c_char,
                data.len(),
                0,
                fh,
            ),
            data.len() as c_int
        );
        assert_eq!(jf_release(fx.ctx, p.as_ptr(), fh), 0);
    }

    #[test]
    fn create_write_release_encrypts_and_round_trips() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/note.md", b"hello world");

        let disk = fx.root().join("diary/note.md.age");
        assert!(disk.is_file(), "a new file lands encrypted (.age) on disk");
        assert!(
            std::fs::read(&disk)
                .unwrap()
                .starts_with(b"age-encryption.org/")
        );
        assert_eq!(fx.read_disk(&disk), b"hello world");
    }

    #[test]
    fn open_reads_at_offset() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/note.md", b"hello world");

        let p = cpath("/diary/note.md");
        let mut fh = 0u64;
        assert_eq!(jf_open(fx.ctx, p.as_ptr(), libc::O_RDONLY, &mut fh), 0);
        let mut buf = [0u8; 5];
        let n = jf_read(
            fx.ctx,
            p.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            5,
            6,
            fh,
        );
        assert_eq!(n, 5);
        assert_eq!(&buf, b"world");
        assert_eq!(jf_release(fx.ctx, p.as_ptr(), fh), 0);
    }

    #[test]
    fn write_past_end_zero_fills_the_gap() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        let p = cpath("/diary/sparse.md");
        let mut fh = 0u64;
        assert_eq!(
            jf_create(fx.ctx, p.as_ptr(), 0o644, libc::O_WRONLY, &mut fh),
            0
        );
        assert_eq!(
            jf_write(
                fx.ctx,
                p.as_ptr(),
                b"AB".as_ptr() as *const c_char,
                2,
                3,
                fh
            ),
            2
        );
        assert_eq!(jf_release(fx.ctx, p.as_ptr(), fh), 0);
        assert_eq!(
            fx.read_disk(&fx.root().join("diary/sparse.md.age")),
            b"\0\0\0AB"
        );
    }

    #[test]
    fn truncate_shrinks_with_open_handle() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/note.md", b"hello world");

        let p = cpath("/diary/note.md");
        let mut fh = 0u64;
        assert_eq!(jf_open(fx.ctx, p.as_ptr(), libc::O_RDWR, &mut fh), 0);
        assert_eq!(jf_truncate(fx.ctx, p.as_ptr(), 5, fh, 1), 0);
        assert_eq!(jf_release(fx.ctx, p.as_ptr(), fh), 0);
        assert_eq!(fx.read_disk(&fx.root().join("diary/note.md.age")), b"hello");
    }

    #[test]
    fn truncate_shrinks_without_open_handle() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/note.md", b"hello world");

        assert_eq!(
            jf_truncate(fx.ctx, cpath("/diary/note.md").as_ptr(), 3, 0, 0),
            0
        );
        assert_eq!(fx.read_disk(&fx.root().join("diary/note.md.age")), b"hel");
    }

    #[test]
    fn rename_preserves_encryption_suffix() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/a.md", b"body");

        assert_eq!(
            jf_rename(
                fx.ctx,
                cpath("/diary/a.md").as_ptr(),
                cpath("/diary/b.md").as_ptr(),
                0,
            ),
            0
        );
        assert!(!fx.root().join("diary/a.md.age").exists());
        let moved = fx.root().join("diary/b.md.age");
        assert!(moved.is_file());
        assert_eq!(fx.read_disk(&moved), b"body");
    }

    #[test]
    fn rename_noreplace_refuses_existing_target() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/a.md", b"aaa");
        write_new(&fx, "/diary/b.md", b"bbb");

        assert_eq!(
            jf_rename(
                fx.ctx,
                cpath("/diary/a.md").as_ptr(),
                cpath("/diary/b.md").as_ptr(),
                RENAME_NOREPLACE,
            ),
            -libc::EEXIST
        );
        assert_eq!(fx.read_disk(&fx.root().join("diary/a.md.age")), b"aaa");
        assert_eq!(fx.read_disk(&fx.root().join("diary/b.md.age")), b"bbb");
    }

    #[test]
    fn unlink_removes_file() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/note.md", b"x");

        let p = cpath("/diary/note.md");
        assert_eq!(jf_unlink(fx.ctx, p.as_ptr()), 0);
        assert!(!fx.root().join("diary/note.md.age").exists());
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        assert_eq!(jf_getattr(fx.ctx, p.as_ptr(), &mut st), -libc::ENOENT);
    }

    #[test]
    fn getattr_reports_plaintext_size() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/note.md", b"hello world");

        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        assert_eq!(
            jf_getattr(fx.ctx, cpath("/diary/note.md").as_ptr(), &mut st),
            0
        );
        assert_eq!(st.st_size, 11); // plaintext length, not the larger ciphertext
    }

    #[test]
    fn age_metadata_is_inaccessible() {
        let fx = Fixture::new();
        assert!(fx.root().join(".age/devices.toml").is_file());

        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        assert_eq!(
            jf_getattr(fx.ctx, cpath("/.age/devices.toml").as_ptr(), &mut st),
            -libc::ENOENT
        );
        let mut fh = 0u64;
        assert_eq!(
            jf_open(
                fx.ctx,
                cpath("/.age/devices.toml").as_ptr(),
                libc::O_RDONLY,
                &mut fh,
            ),
            -libc::ENOENT
        );
    }

    #[test]
    fn getattr_reports_directory_mtime() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        assert_eq!(jf_getattr(fx.ctx, cpath("/diary").as_ptr(), &mut st), 0);
        assert!(st.st_mtime > 0, "directories should carry their real mtime");
    }

    #[test]
    fn rmdir_clears_rejected_system_state_from_a_visually_empty_folder() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        std::fs::create_dir(fx.root().join("diary/.Spotlight-V100")).unwrap();

        assert!(visible_entries(&fx.root().join("diary")).is_empty());
        assert_eq!(jf_rmdir(fx.ctx, cpath("/diary").as_ptr()), 0);
        assert!(!fx.root().join("diary").exists());
    }

    #[test]
    fn rmdir_keeps_a_folder_with_visible_contents() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/note.md", b"x");

        assert_eq!(jf_rmdir(fx.ctx, cpath("/diary").as_ptr()), -libc::ENOTEMPTY);
        assert!(fx.root().join("diary").is_dir());
    }

    #[test]
    fn create_rejects_system_state() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        let mut fh = 0u64;
        assert_eq!(
            jf_create(
                fx.ctx,
                cpath("/diary/.Spotlight-V100").as_ptr(),
                0o644,
                libc::O_WRONLY,
                &mut fh,
            ),
            -libc::ENOENT
        );
        assert_eq!(
            jf_create(
                fx.ctx,
                cpath("/diary/._note.md").as_ptr(),
                0o644,
                libc::O_WRONLY,
                &mut fh,
            ),
            -libc::ENOENT
        );
    }

    #[test]
    fn benign_metadata_files_pass_through_plaintext() {
        let fx = Fixture::new();
        fx.mkdir_p("diary");
        write_new(&fx, "/diary/.DS_Store", b"finder state");

        let disk = fx.root().join("diary/.DS_Store");
        assert!(disk.is_file());
        assert_eq!(std::fs::read(disk).unwrap(), b"finder state");
    }

    #[test]
    fn visible_entries_hides_age_and_rejected_state_keeps_trash_strips_content_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".age")).unwrap();
        std::fs::create_dir(root.join(".trash")).unwrap();
        std::fs::create_dir(root.join("diary")).unwrap();
        std::fs::write(root.join(".DS_Store"), b"").unwrap();
        std::fs::create_dir(root.join(".Spotlight-V100")).unwrap();
        std::fs::write(root.join("diary/x.md.age"), b"").unwrap();
        std::fs::write(root.join("diary/notes.age"), b"").unwrap();

        let top = visible_entries(root);
        assert!(top.contains(&OsString::from(".trash")));
        assert!(top.contains(&OsString::from("diary")));
        assert!(top.contains(&OsString::from(".DS_Store")));
        assert!(!top.contains(&OsString::from(".age")));
        assert!(!top.contains(&OsString::from(".Spotlight-V100")));

        let diary = visible_entries(&root.join("diary"));
        assert!(diary.contains(&OsString::from("x.md")));
        assert!(diary.contains(&OsString::from("notes.age")));
    }
}
