use notema_core::AppResult;
use notema_storage::{JournalStore, StoreFileEncoding};
use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr};
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::UNIX_EPOCH;
use zeroize::Zeroizing;

use crate::path_policy::{
    BackingFile, backing_for_new_file, existing_file, is_directory, is_protected_path,
    is_rejected_system_name, visible_entries, with_age,
};

/// An open file: the whole file buffered as plaintext bytes, its on-disk path
/// (with `.age` when encrypted), and whether it was opened writable / is dirty.
struct Handle {
    on_disk: PathBuf,
    encoding: StoreFileEncoding,
    buf: Zeroizing<Vec<u8>>,
    deleted: bool,
    dirty: bool,
    writable: bool,
}

/// Per-mount state, reached from every callback via libfuse's `private_data`.
/// A single dispatch thread (`-s`) serializes access, but the mutex keeps it
/// sound and simple.
pub(super) struct Ctx {
    store: JournalStore,
    root: PathBuf,
    inner: Mutex<Inner>,
    uid: u32,
    gid: u32,
}

struct Inner {
    handles: HashMap<u64, Handle>,
    next_fh: u64,
}

/// `rename` flags as delivered by the FUSE protocol (kernel values, the same
/// across libfuse/macFUSE/fuse-t). `libc`'s `RENAME_*` constants differ per
/// platform, so we pin the wire values here.
pub(super) const RENAME_NOREPLACE: u32 = 1;
const RENAME_EXCHANGE: u32 = 2;

// --- path helpers -----------------------------------------------------------

pub(super) fn ctx_ref<'a>(ptr: *mut c_void) -> &'a Ctx {
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

impl Ctx {
    pub(super) fn new(store: JournalStore) -> Self {
        let root = store.paths().journal_root.clone();
        Self {
            store,
            root,
            inner: Mutex::new(Inner {
                handles: HashMap::new(),
                next_fh: 1,
            }),
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        }
    }

    #[cfg(test)]
    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(test)]
    pub(super) fn store(&self) -> &JournalStore {
        &self.store
    }

    /// Lock the mutable state, recovering from poisoning: a panic caught at the
    /// FFI boundary (see [`guard`]) can poison the lock, but the protected data
    /// stays structurally valid, so we keep serving the mount rather than
    /// wedging every later call into a panic.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Current plaintext length of an on-disk file, preferring an open handle's
    /// live buffer so `stat` stays consistent mid-edit.
    fn file_size(&self, file: &BackingFile) -> AppResult<u64> {
        let inner = self.lock();
        if let Some(handle) = inner.handles.values().find(|h| h.on_disk == file.path) {
            return Ok(handle.buf.len() as u64);
        }
        drop(inner);
        Ok(self.store.read_store_file(&file.path, file.encoding)?.len() as u64)
    }

    /// Re-encrypt and write back a dirty handle's buffer; no-op for clean or
    /// read-only handles.
    fn commit(&self, fh: u64) -> Result<(), c_int> {
        let inner = self.lock();
        let Some(handle) = inner.handles.get(&fh) else {
            return Ok(());
        };
        if handle.deleted {
            return Ok(());
        }
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
        unsafe { super::bridge_fill(filler, buf, name.as_ptr()) };
    }
    let base = base_of(ctx, path);
    let entries = match visible_entries(&base) {
        Ok(entries) => entries,
        Err(err) => return errno(&err),
    };
    for name in entries {
        if let Ok(name) = CString::new(name.as_bytes()) {
            unsafe { super::bridge_fill(filler, buf, name.as_ptr()) };
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
        Zeroizing::new(Vec::new())
    } else {
        match ctx.store.read_store_file(&file.path, file.encoding) {
            Ok(bytes) => Zeroizing::new(bytes),
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
            deleted: false,
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
            buf: Zeroizing::new(Vec::new()),
            deleted: false,
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
    if off < 0 {
        return -libc::EINVAL;
    }
    let start = (off as usize).min(handle.buf.len());
    let end = start.saturating_add(size).min(handle.buf.len());
    let slice = &handle.buf[start..end];
    let Ok(written) = c_int::try_from(slice.len()) else {
        return -libc::EFBIG;
    };
    unsafe { std::ptr::copy_nonoverlapping(slice.as_ptr(), buf as *mut u8, slice.len()) };
    written
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
    if off < 0 {
        return -libc::EINVAL;
    }
    let Ok(written) = c_int::try_from(size) else {
        return -libc::EFBIG;
    };
    let start = off as usize;
    let Some(end) = start.checked_add(size) else {
        return -libc::EFBIG;
    };
    if handle.buf.len() < end {
        handle.buf.resize(end, 0);
    }
    let data = unsafe { std::slice::from_raw_parts(buf as *const u8, size) };
    handle.buf[start..end].copy_from_slice(data);
    handle.dirty = true;
    written
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
    if size < 0 {
        return -libc::EINVAL;
    }
    let size = size as usize;
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
            let mut inner = ctx.lock();
            for handle in inner
                .handles
                .values_mut()
                .filter(|handle| handle.on_disk == file.path)
            {
                handle.deleted = true;
                handle.dirty = false;
            }
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
        let visually_empty = visible_entries(&dir).is_ok_and(|entries| entries.is_empty());
        if err.raw_os_error() == Some(libc::ENOTEMPTY) && visually_empty {
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
    if from_disk == to_disk {
        return 0;
    }
    match std::fs::rename(&from_disk, &to_disk) {
        Ok(()) => {
            let mut inner = ctx.lock();
            for handle in inner.handles.values_mut() {
                if handle.on_disk == to_disk {
                    handle.deleted = true;
                    handle.dirty = false;
                } else if handle.on_disk == from_disk {
                    handle.on_disk = to_disk.clone();
                } else if let Ok(relative) = handle.on_disk.strip_prefix(&from_disk) {
                    handle.on_disk = to_disk.join(relative);
                }
            }
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
