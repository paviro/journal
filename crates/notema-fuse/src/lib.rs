//! A FUSE filesystem that exposes an encrypted journal as an ordinary, decrypted
//! directory tree, re-encrypting everything written back. The store stays
//! encrypted on disk; plaintext only ever lives in this process's memory.
//!
//! The mount mirrors the on-disk layout with the encryption suffix stripped from
//! file names: an entry stored as `…/2026/07/09/<id>.md.age` appears as
//! `…/2026/07/09/<id>.md`, and an encrypted asset `<id>.assets/photo.jpg.age`
//! appears as `<id>.assets/photo.jpg`. Journal content is decrypted on read and
//! re-encrypted on write; system metadata and unrelated files pass through
//! unchanged.
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

use anyhow::Result as AppResult;
use notema_storage::JournalStore;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

mod ops;
mod path_policy;

unsafe extern "C" {
    fn notema_fuse_run(
        mountpoint: *const c_char,
        context: *mut c_void,
        volname: *const c_char,
    ) -> c_int;
    pub(crate) fn bridge_fill(filler: *mut c_void, buf: *mut c_void, name: *const c_char) -> c_int;
}

/// Mount `store` (already unlocked) at `mountpoint`, exposing its entries and
/// assets as a decrypted, writable tree. Blocks until the filesystem is
/// unmounted. The caller must unlock the store's identity first.
pub fn mount(store: JournalStore, mountpoint: &Path) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    ensure_macfuse_loaded();

    let ctx = Box::new(ops::Ctx::new(store));
    let ctx_ptr = Box::into_raw(ctx) as *mut c_void;

    let mountpoint = CString::new(mountpoint.as_os_str().as_bytes())?;
    // macFUSE/fuse-t show a Finder volume regardless of the mount path; name it.
    let volname = if cfg!(target_os = "macos") {
        Some(CString::new("Journals").unwrap())
    } else {
        None
    };
    let volname_ptr = volname.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());

    // SAFETY: All C strings outlive the blocking call and `ctx_ptr` owns a live
    // `Ctx` until the event loop returns.
    let _status = unsafe { notema_fuse_run(mountpoint.as_ptr(), ctx_ptr, volname_ptr) };

    // Reclaim the leaked context now that the event loop has returned.
    // SAFETY: `ctx_ptr` came from `Box::into_raw` above and libfuse has stopped
    // using it now that `notema_fuse_run` returned.
    drop(unsafe { Box::from_raw(ctx_ptr as *mut ops::Ctx) });

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

#[cfg(test)]
mod tests;
