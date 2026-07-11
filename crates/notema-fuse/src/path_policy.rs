use notema_storage::StoreFileEncoding;
use std::ffi::{CStr, OsStr, OsString};
use std::os::raw::c_char;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BackingFile {
    pub(super) path: PathBuf,
    pub(super) encoding: StoreFileEncoding,
}

/// Append the age extension: `x.md` -> `x.md.age`.
pub(super) fn with_age(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".age");
    PathBuf::from(name)
}

pub(super) fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| {
            let ty = meta.file_type();
            ty.is_file() && !ty.is_symlink()
        })
        .unwrap_or(false)
}

pub(super) fn is_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| {
            let ty = meta.file_type();
            ty.is_dir() && !ty.is_symlink()
        })
        .unwrap_or(false)
}

/// Resolve a mounted file's actual on-disk path: the encrypted `<base>.age` if
/// it exists, else the plain `<base>`, else `None` when neither exists.
pub(super) fn existing_file(base: &Path) -> Option<BackingFile> {
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

pub(super) fn backing_for_new_file(base: PathBuf) -> BackingFile {
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

pub(super) fn is_rejected_system_name(name: &OsStr) -> bool {
    component_is_rejected_system_state(name.as_bytes())
}

pub(super) fn is_protected_path(path: *const c_char) -> bool {
    !is_safe_mounted_path(path) || is_rejected_system_path(path)
}

pub(super) fn visible_entries(base: &Path) -> std::io::Result<Vec<OsString>> {
    let entries = std::fs::read_dir(base)?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        let disk_name = entry.file_name();
        if is_rejected_system_name(&disk_name) {
            continue;
        }
        let path = entry.path();
        if disk_name
            .to_str()
            .is_some_and(|name| name.ends_with(".age"))
        {
            names.push(mounted_name_for_backing(&path, &disk_name));
        } else if existing_file(&path).is_some_and(|file| file.encoding == StoreFileEncoding::Plain)
            || is_directory(&path)
        {
            names.push(disk_name);
        }
    }
    Ok(names)
}

pub(super) fn mounted_name(disk_name: &OsStr) -> OsString {
    match disk_name.to_str() {
        Some(s) => OsString::from(s.strip_suffix(".age").unwrap_or(s)),
        None => disk_name.to_os_string(),
    }
}
