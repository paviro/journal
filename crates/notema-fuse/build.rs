// Build the C bridge (bridge.c) and link libfuse3. The bridge defines the
// `fuse_operations` struct in C — so its layout always matches the linked
// libfuse headers — and forwards every callback to the Rust functions in lib.rs.
//
// libfuse3 is provided by libfuse3-dev on Linux and by macFUSE or fuse-t on
// macOS; all expose the same high-level API, so one bridge covers every target.
fn main() {
    // No version gate: the API level is pinned by FUSE_USE_VERSION below, and
    // some backends (fuse-t) advertise their own version in fuse3.pc rather than
    // libfuse's, so an `atleast_version("3")` check would wrongly reject them.
    let lib = pkg_config::Config::new().probe("fuse3").expect(
        "libfuse3 is required to build the `fuse` feature \
             (Linux: install libfuse3-dev; macOS: install macFUSE or fuse-t)",
    );

    let mut build = cc::Build::new();
    build.file("bridge.c");
    build.define("_FILE_OFFSET_BITS", "64");
    build.define("FUSE_USE_VERSION", "31");
    for path in &lib.include_paths {
        build.include(path);
    }
    build.compile("notema_fuse_bridge");

    // pkg-config's link-lib/link-search directives (emitted above) propagate to
    // the final binary, but rpath link-args from a dependency build script do
    // not — the binary crate's build.rs embeds the runtime rpath there. On macOS,
    // running this crate's own test binary standalone therefore needs
    // `DYLD_LIBRARY_PATH=/usr/local/lib` (libfuse3 has an `@rpath` install name).
    let _ = &lib.link_paths;
    println!("cargo:rerun-if-changed=bridge.c");
}
