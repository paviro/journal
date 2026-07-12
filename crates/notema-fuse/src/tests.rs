use super::ops::{Ctx, RENAME_NOREPLACE, ctx_ref as raw_ctx_ref};
use crate::path_policy::{
    BackingFile, existing_file, is_rejected_system_name, mounted_name, visible_entries, with_age,
};
use notema_encryption::SecretString;
use notema_storage::{JournalStore, StoreFileEncoding};
use std::ffi::{CString, OsStr, OsString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};

fn ctx_ref<'a>(ctx: *mut c_void) -> &'a Ctx {
    // SAFETY: Every test context comes from `Box::into_raw` in `Harness` and
    // remains owned until the harness is dropped.
    unsafe { raw_ctx_ref(ctx) }
}

fn jf_create(
    ctx: *mut c_void,
    path: *const c_char,
    mode: u32,
    flags: c_int,
    fh_out: *mut u64,
) -> c_int {
    // SAFETY: Test callers pass live CStrings, context pointers, and output slots.
    unsafe { super::ops::jf_create(ctx, path, mode, flags, fh_out) }
}

fn jf_open(ctx: *mut c_void, path: *const c_char, flags: c_int, fh_out: *mut u64) -> c_int {
    // SAFETY: Test callers pass live CStrings, context pointers, and output slots.
    unsafe { super::ops::jf_open(ctx, path, flags, fh_out) }
}

fn jf_read(
    ctx: *mut c_void,
    path: *const c_char,
    buf: *mut c_char,
    size: usize,
    offset: i64,
    fh: u64,
) -> c_int {
    // SAFETY: Test callers size the writable buffer to at least `size` bytes.
    unsafe { super::ops::jf_read(ctx, path, buf, size, offset, fh) }
}

fn jf_write(
    ctx: *mut c_void,
    path: *const c_char,
    buf: *const c_char,
    size: usize,
    offset: i64,
    fh: u64,
) -> c_int {
    // SAFETY: Test callers keep the input buffer alive for `size` bytes.
    unsafe { super::ops::jf_write(ctx, path, buf, size, offset, fh) }
}

fn jf_truncate(ctx: *mut c_void, path: *const c_char, size: i64, fh: u64, has_fh: c_int) -> c_int {
    // SAFETY: Test callers pass a live harness context and CString path.
    unsafe { super::ops::jf_truncate(ctx, path, size, fh, has_fh) }
}

fn jf_release(ctx: *mut c_void, path: *const c_char, fh: u64) -> c_int {
    // SAFETY: Test callers pass a live harness context and CString path.
    unsafe { super::ops::jf_release(ctx, path, fh) }
}

fn jf_rename(ctx: *mut c_void, from: *const c_char, to: *const c_char, flags: u32) -> c_int {
    // SAFETY: Test callers keep both CString paths and the context alive.
    unsafe { super::ops::jf_rename(ctx, from, to, flags) }
}

fn jf_unlink(ctx: *mut c_void, path: *const c_char) -> c_int {
    // SAFETY: Test callers pass a live harness context and CString path.
    unsafe { super::ops::jf_unlink(ctx, path) }
}

fn jf_getattr(ctx: *mut c_void, path: *const c_char, stat: *mut libc::stat) -> c_int {
    // SAFETY: Test callers pass a live context/path and writable `stat`.
    unsafe { super::ops::jf_getattr(ctx, path, stat) }
}

fn jf_rmdir(ctx: *mut c_void, path: *const c_char) -> c_int {
    // SAFETY: Test callers pass a live harness context and CString path.
    unsafe { super::ops::jf_rmdir(ctx, path) }
}

fn empty_stat() -> libc::stat {
    // SAFETY: `libc::stat` is a plain C output structure and an all-zero value
    // is a valid initialized buffer for `getattr` to fill.
    unsafe { std::mem::zeroed() }
}

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
        let ctx = Box::new(Ctx::new(store));
        Self {
            _dir: dir,
            ctx: Box::into_raw(ctx) as *mut c_void,
        }
    }

    fn root(&self) -> PathBuf {
        ctx_ref(self.ctx).root().to_path_buf()
    }

    /// Decrypt an on-disk store file back to plaintext.
    fn read_disk(&self, disk: &Path) -> Vec<u8> {
        ctx_ref(self.ctx)
            .store()
            .read_store_file(disk, StoreFileEncoding::Encrypted)
            .unwrap()
    }

    fn mkdir_p(&self, rel: &str) {
        std::fs::create_dir_all(self.root().join(rel)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // SAFETY: `self.ctx` was created exactly once with `Box::into_raw`, and
        // no test callback can outlive the fixture borrow.
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
fn shadowed_age_alias_is_not_addressable() {
    let dir = tempfile::tempdir().unwrap();
    // The raw ciphertext name of an encrypted entry is hidden…
    let entry = dir.path().join("x.md.age");
    std::fs::write(&entry, b"age-encryption.org/ ...").unwrap();
    assert_eq!(existing_file(&dir.path().join("x.md.age")), None);
    // …but a plain sidecar like foo.txt.age stays a real, resolvable file.
    let plain = dir.path().join("foo.txt.age");
    std::fs::write(&plain, b"ct").unwrap();
    assert_eq!(
        existing_file(&plain),
        Some(BackingFile {
            path: plain,
            encoding: StoreFileEncoding::Plain,
        })
    );
}

#[test]
fn writing_through_age_alias_cannot_overwrite_ciphertext_with_plaintext() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    write_new(&fx, "/diary/note.md", b"secret body");
    let disk = fx.root().join("diary/note.md.age");
    let ciphertext = std::fs::read(&disk).unwrap();

    // getattr, open, and create through the raw .age name must all be refused,
    // so no writable handle onto the ciphertext can ever exist.
    let alias = cpath("/diary/note.md.age");
    let mut stat = empty_stat();
    assert_eq!(jf_getattr(fx.ctx, alias.as_ptr(), &mut stat), -libc::ENOENT);
    let mut fh = 0u64;
    assert_eq!(
        jf_open(fx.ctx, alias.as_ptr(), libc::O_RDWR, &mut fh),
        -libc::ENOENT
    );
    assert_eq!(
        jf_create(fx.ctx, alias.as_ptr(), 0o644, libc::O_WRONLY, &mut fh),
        -libc::EACCES
    );

    // The ciphertext is untouched and still decrypts.
    assert_eq!(std::fs::read(&disk).unwrap(), ciphertext);
    assert_eq!(fx.read_disk(&disk), b"secret body");
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
fn negative_offsets_are_rejected() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    write_new(&fx, "/diary/note.md", b"hello");

    let p = cpath("/diary/note.md");
    let mut fh = 0u64;
    assert_eq!(jf_open(fx.ctx, p.as_ptr(), libc::O_RDWR, &mut fh), 0);
    let mut buf = [0u8; 1];
    assert_eq!(
        jf_read(
            fx.ctx,
            p.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            1,
            -1,
            fh,
        ),
        -libc::EINVAL
    );
    assert_eq!(
        jf_write(
            fx.ctx,
            p.as_ptr(),
            b"x".as_ptr() as *const c_char,
            1,
            -1,
            fh,
        ),
        -libc::EINVAL
    );
    assert_eq!(jf_truncate(fx.ctx, p.as_ptr(), -1, fh, 1), -libc::EINVAL);
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
fn rename_open_file_moves_later_writeback_to_new_path() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    write_new(&fx, "/diary/a.md", b"old body");

    let p = cpath("/diary/a.md");
    let mut fh = 0u64;
    assert_eq!(
        jf_open(fx.ctx, p.as_ptr(), libc::O_RDWR | libc::O_TRUNC, &mut fh,),
        0
    );
    assert_eq!(
        jf_rename(
            fx.ctx,
            cpath("/diary/a.md").as_ptr(),
            cpath("/diary/b.md").as_ptr(),
            0,
        ),
        0
    );
    assert_eq!(
        jf_write(
            fx.ctx,
            p.as_ptr(),
            b"new body".as_ptr() as *const c_char,
            8,
            0,
            fh,
        ),
        8
    );
    assert_eq!(jf_release(fx.ctx, p.as_ptr(), fh), 0);
    assert!(!fx.root().join("diary/a.md.age").exists());
    assert_eq!(fx.read_disk(&fx.root().join("diary/b.md.age")), b"new body");
}

#[test]
fn unlink_removes_file() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    write_new(&fx, "/diary/note.md", b"x");

    let p = cpath("/diary/note.md");
    assert_eq!(jf_unlink(fx.ctx, p.as_ptr()), 0);
    assert!(!fx.root().join("diary/note.md.age").exists());
    let mut st = empty_stat();
    assert_eq!(jf_getattr(fx.ctx, p.as_ptr(), &mut st), -libc::ENOENT);
}

#[test]
fn unlink_open_file_release_does_not_resurrect_it() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    write_new(&fx, "/diary/note.md", b"old body");

    let p = cpath("/diary/note.md");
    let mut fh = 0u64;
    assert_eq!(jf_open(fx.ctx, p.as_ptr(), libc::O_RDWR, &mut fh), 0);
    assert_eq!(
        jf_write(
            fx.ctx,
            p.as_ptr(),
            b"new".as_ptr() as *const c_char,
            3,
            0,
            fh,
        ),
        3
    );
    assert_eq!(jf_unlink(fx.ctx, p.as_ptr()), 0);
    assert_eq!(jf_release(fx.ctx, p.as_ptr(), fh), 0);
    assert!(!fx.root().join("diary/note.md.age").exists());
}

#[test]
fn getattr_reports_plaintext_size() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    write_new(&fx, "/diary/note.md", b"hello world");

    let mut st = empty_stat();
    assert_eq!(
        jf_getattr(fx.ctx, cpath("/diary/note.md").as_ptr(), &mut st),
        0
    );
    assert_eq!(st.st_size, 11); // plaintext length, not the larger ciphertext
}

#[test]
fn getattr_reflects_external_size_changes() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    write_new(&fx, "/diary/note.md", b"abc");

    let disk = fx.root().join("diary/note.md.age");
    ctx_ref(fx.ctx)
        .store()
        .write_store_file(&disk, StoreFileEncoding::Encrypted, b"abcdef")
        .unwrap();

    let mut st = empty_stat();
    assert_eq!(
        jf_getattr(fx.ctx, cpath("/diary/note.md").as_ptr(), &mut st),
        0
    );
    assert_eq!(st.st_size, 6);
}

#[test]
fn age_metadata_passes_through_plaintext() {
    let fx = Fixture::new();
    let disk = fx.root().join(".age/devices.toml");
    assert!(disk.is_file());
    let raw = std::fs::read(&disk).unwrap();

    let mut st = empty_stat();
    assert_eq!(
        jf_getattr(fx.ctx, cpath("/.age/devices.toml").as_ptr(), &mut st),
        0
    );
    assert_eq!(st.st_size, raw.len() as libc::off_t);
    let mut fh = 0u64;
    assert_eq!(
        jf_open(
            fx.ctx,
            cpath("/.age/devices.toml").as_ptr(),
            libc::O_RDONLY,
            &mut fh,
        ),
        0
    );
    let mut buf = vec![0u8; raw.len().min(64)];
    assert_eq!(
        jf_read(
            fx.ctx,
            cpath("/.age/devices.toml").as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            0,
            fh,
        ),
        buf.len() as c_int
    );
    assert_eq!(&buf, &raw[..buf.len()]);
    assert_eq!(
        jf_release(fx.ctx, cpath("/.age/devices.toml").as_ptr(), fh),
        0
    );
}

#[test]
fn traversal_and_symlinks_are_inaccessible() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    write_new(&fx, "/diary/target.md", b"x");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        fx.root().join("diary/target.md.age"),
        fx.root().join("diary/link.md.age"),
    )
    .unwrap();

    let mut st = empty_stat();
    assert_eq!(
        jf_getattr(
            fx.ctx,
            cpath("/diary/../.age/devices.toml").as_ptr(),
            &mut st
        ),
        -libc::ENOENT
    );
    #[cfg(unix)]
    assert_eq!(
        jf_getattr(fx.ctx, cpath("/diary/link.md").as_ptr(), &mut st),
        -libc::ENOENT
    );
}

#[test]
fn getattr_reports_directory_mtime() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    let mut st = empty_stat();
    assert_eq!(jf_getattr(fx.ctx, cpath("/diary").as_ptr(), &mut st), 0);
    assert!(st.st_mtime > 0, "directories should carry their real mtime");
}

#[test]
fn rmdir_clears_rejected_system_state_from_a_visually_empty_folder() {
    let fx = Fixture::new();
    fx.mkdir_p("diary");
    std::fs::create_dir(fx.root().join("diary/.Spotlight-V100")).unwrap();

    assert!(
        visible_entries(&fx.root().join("diary"))
            .unwrap()
            .is_empty()
    );
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

    let top = visible_entries(root).unwrap();
    assert!(top.contains(&OsString::from(".trash")));
    assert!(top.contains(&OsString::from("diary")));
    assert!(top.contains(&OsString::from(".DS_Store")));
    assert!(top.contains(&OsString::from(".age")));
    assert!(!top.contains(&OsString::from(".Spotlight-V100")));

    let diary = visible_entries(&root.join("diary")).unwrap();
    assert!(diary.contains(&OsString::from("x.md")));
    assert!(diary.contains(&OsString::from("notes.age")));
}
