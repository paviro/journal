// C bridge between libfuse's high-level API and the Rust callbacks in lib.rs.
//
// Defining `fuse_operations` here (rather than in Rust) means the struct layout
// and callback signatures always match whatever libfuse3 the build links against.
// Each operation forwards to a `jf_*` function implemented in Rust, passing the
// per-mount context (a leaked `Box<Ctx>`) fetched from libfuse's private_data.
// fuse_file_info stays entirely on this side: we hand Rust only the open flags
// and the opaque `fh` handle it set, so Rust never depends on that struct's
// layout. Directory entries are emitted through `bridge_fill` for the same reason.

#include <fuse.h>
#include <stdio.h>
#include <string.h>
#include <sys/statvfs.h>

// Implemented in Rust (lib.rs). All take the context pointer first and return
// 0 on success or a negative errno.
extern int jf_getattr(void *ctx, const char *path, struct stat *st);
extern int jf_readdir(void *ctx, const char *path, void *buf, void *filler);
extern int jf_open(void *ctx, const char *path, int flags, unsigned long long *fh_out);
extern int jf_create(void *ctx, const char *path, unsigned int mode, int flags,
                     unsigned long long *fh_out);
extern int jf_read(void *ctx, const char *path, char *buf, unsigned long size, long off,
                   unsigned long long fh);
extern int jf_write(void *ctx, const char *path, const char *buf, unsigned long size, long off,
                    unsigned long long fh);
extern int jf_truncate(void *ctx, const char *path, long size, unsigned long long fh, int has_fh);
extern int jf_unlink(void *ctx, const char *path);
extern int jf_mkdir(void *ctx, const char *path, unsigned int mode);
extern int jf_rmdir(void *ctx, const char *path);
extern int jf_rename(void *ctx, const char *from, const char *to, unsigned int flags);
extern int jf_release(void *ctx, const char *path, unsigned long long fh);
extern int jf_flush(void *ctx, const char *path, unsigned long long fh);
extern int jf_statfs(void *ctx, const char *path, struct statvfs *st);

// Let Rust emit a directory entry without needing libfuse's fill-dir signature.
int bridge_fill(void *filler, void *buf, const char *name) {
    return ((fuse_fill_dir_t)filler)(buf, name, NULL, 0, 0);
}

static void *ctx(void) { return fuse_get_context()->private_data; }

static int b_getattr(const char *p, struct stat *st, struct fuse_file_info *fi) {
    (void)fi;
    return jf_getattr(ctx(), p, st);
}
static int b_readdir(const char *p, void *buf, fuse_fill_dir_t filler, off_t off,
                     struct fuse_file_info *fi, enum fuse_readdir_flags flags) {
    (void)off;
    (void)fi;
    (void)flags;
    return jf_readdir(ctx(), p, buf, (void *)filler);
}
static int b_open(const char *p, struct fuse_file_info *fi) {
    unsigned long long fh = 0;
    int r = jf_open(ctx(), p, fi->flags, &fh);
    if (r == 0)
        fi->fh = fh;
    return r;
}
static int b_create(const char *p, mode_t mode, struct fuse_file_info *fi) {
    unsigned long long fh = 0;
    int r = jf_create(ctx(), p, mode, fi->flags, &fh);
    if (r == 0)
        fi->fh = fh;
    return r;
}
static int b_read(const char *p, char *buf, size_t size, off_t off, struct fuse_file_info *fi) {
    return jf_read(ctx(), p, buf, size, off, fi->fh);
}
static int b_write(const char *p, const char *buf, size_t size, off_t off,
                   struct fuse_file_info *fi) {
    return jf_write(ctx(), p, buf, size, off, fi->fh);
}
static int b_truncate(const char *p, off_t size, struct fuse_file_info *fi) {
    return jf_truncate(ctx(), p, size, fi ? fi->fh : 0, fi ? 1 : 0);
}
static int b_unlink(const char *p) { return jf_unlink(ctx(), p); }
static int b_mkdir(const char *p, mode_t mode) { return jf_mkdir(ctx(), p, mode); }
static int b_rmdir(const char *p) { return jf_rmdir(ctx(), p); }
static int b_rename(const char *from, const char *to, unsigned int flags) {
    return jf_rename(ctx(), from, to, flags);
}
static int b_release(const char *p, struct fuse_file_info *fi) {
    return jf_release(ctx(), p, fi->fh);
}
static int b_flush(const char *p, struct fuse_file_info *fi) {
    return jf_flush(ctx(), p, fi->fh);
}
static int b_fsync(const char *p, int datasync, struct fuse_file_info *fi) {
    (void)datasync;
    return jf_flush(ctx(), p, fi->fh);
}
static int b_statfs(const char *p, struct statvfs *st) { return jf_statfs(ctx(), p, st); }
// Accept metadata changes editors make on save so the write itself isn't rejected.
static int b_chmod(const char *p, mode_t m, struct fuse_file_info *fi) {
    (void)p;
    (void)m;
    (void)fi;
    return 0;
}
static int b_chown(const char *p, uid_t u, gid_t g, struct fuse_file_info *fi) {
    (void)p;
    (void)u;
    (void)g;
    (void)fi;
    return 0;
}
static int b_utimens(const char *p, const struct timespec tv[2], struct fuse_file_info *fi) {
    (void)p;
    (void)tv;
    (void)fi;
    return 0;
}

static struct fuse_operations ops = {
    .getattr = b_getattr,
    .readdir = b_readdir,
    .open = b_open,
    .create = b_create,
    .read = b_read,
    .write = b_write,
    .truncate = b_truncate,
    .unlink = b_unlink,
    .mkdir = b_mkdir,
    .rmdir = b_rmdir,
    .rename = b_rename,
    .release = b_release,
    .flush = b_flush,
    .fsync = b_fsync,
    .statfs = b_statfs,
    .chmod = b_chmod,
    .chown = b_chown,
    .utimens = b_utimens,
};

// Mount at `mountpoint`, blocking until unmounted. `context` becomes libfuse's
// private_data (our Rust Ctx). `volname` sets the macOS Finder volume name when
// non-NULL. Returns fuse_main's status (0 on clean exit).
int notema_fuse_run(const char *mountpoint, void *context, const char *volname) {
    char *argv[8];
    int argc = 0;
    char opts[512];
    argv[argc++] = "journal";
    argv[argc++] = (char *)mountpoint;
    argv[argc++] = "-s"; // single-threaded: serialize access to the Rust Ctx
    argv[argc++] = "-f"; // foreground: this call blocks until unmount
    // Zero cache timeouts so store changes made by another process (a second
    // `journal` instance) show up promptly instead of after a cache window.
    // default_permissions lets the kernel enforce our reported ownership/mode.
    snprintf(opts, sizeof(opts),
             "default_permissions,entry_timeout=0,attr_timeout=0,negative_timeout=0");
    if (volname && volname[0]) {
        size_t len = strlen(opts);
        snprintf(opts + len, sizeof(opts) - len, ",volname=%s", volname);
    }
    argv[argc++] = "-o";
    argv[argc++] = opts;
    return fuse_main(argc, argv, &ops, context);
}
