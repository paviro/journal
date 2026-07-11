//! Enumerating an entry's referenced images and the image-number ⇄ digit-key
//! mapping shared by the entry-view labels and keyboard shortcuts.
//!
//! Images live in the entry's sibling `<stem>.assets/` folder, referenced as a
//! lone markdown image on its own line. Parsing is owned by the storage crate
//! ([`notema_storage::sole_stored_image`]) so labels, viewer, and asset
//! cleanup agree on what counts as an image — and thus on its `Image N` number.

use std::path::Path;

use notema_storage::sole_stored_image;

use super::ImageAsset;

/// Enumerate an entry's in-folder images in body order.
pub(crate) fn entry_images(content: &str, entry_path: &Path) -> Vec<ImageAsset> {
    content
        .split('\n')
        .filter_map(|line| sole_image_ref(line, entry_path))
        .map(|(_alt, asset)| asset)
        .collect()
}

/// If a line is exactly a single markdown image inside this entry's `.assets/`
/// folder, return its alt text and stored asset key.
pub(crate) fn sole_image_ref(line: &str, entry_path: &Path) -> Option<(String, ImageAsset)> {
    let (alt, file_name) = sole_stored_image(line, entry_path)?;
    Some((
        alt,
        ImageAsset {
            entry_path: entry_path.to_path_buf(),
            file_name,
        },
    ))
}

/// The digit key that opens the image at `index` (0-based): first nine bind to
/// `1`–`9`, the tenth to `0`, none past that. Inverse of [`image_for_digit`].
pub(crate) fn digit_for_image(index: usize) -> Option<char> {
    match index {
        0..=8 => Some((b'1' + index as u8) as char),
        9 => Some('0'),
        _ => None,
    }
}

/// The image index a digit key opens: `1`–`9` the first nine, `0` the tenth.
/// Inverse of [`digit_for_image`].
pub(crate) fn image_for_digit(ch: char) -> Option<usize> {
    match ch {
        '1'..='9' => Some(ch as usize - '1' as usize),
        '0' => Some(9),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn entry_path_with_asset() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let assets = dir.path().join("2026-07-05T14-30-00-abc123.assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("x9k2.png"), b"img").unwrap();
        let entry_path = dir.path().join("2026-07-05T14-30-00-abc123.md");
        fs::write(&entry_path, b"entry").unwrap();
        (dir, entry_path)
    }

    #[test]
    fn sole_image_ref_builds_asset_key() {
        let (_guard, entry_path) = entry_path_with_asset();
        let line = "![a shot](2026-07-05T14-30-00-abc123.assets/x9k2.png)";

        let (alt, asset) = sole_image_ref(line, &entry_path).expect("should match");
        assert_eq!(alt, "a shot");
        assert_eq!(asset.entry_path, entry_path);
        assert_eq!(asset.file_name, "x9k2.png");
    }

    #[test]
    fn enumerates_images_in_body_order() {
        let (_guard, entry_path) = entry_path_with_asset();
        let assets = entry_path.with_file_name("2026-07-05T14-30-00-abc123.assets");
        fs::write(assets.join("aa11.png"), b"img").unwrap();
        let content = concat!(
            "Intro\n",
            "![first](2026-07-05T14-30-00-abc123.assets/x9k2.png)\n",
            "middle\n",
            "![second](2026-07-05T14-30-00-abc123.assets/aa11.png)\n",
        );

        let images = entry_images(content, &entry_path);

        assert_eq!(images.len(), 2);
        assert_eq!(images[0].file_name, "x9k2.png");
        assert_eq!(images[1].file_name, "aa11.png");
    }

    #[test]
    fn digit_and_image_index_are_inverses() {
        for index in 0..10 {
            let digit = digit_for_image(index).expect("first ten images bind to a digit");
            assert_eq!(image_for_digit(digit), Some(index));
        }
        assert_eq!(digit_for_image(10), None);
        assert_eq!(image_for_digit('a'), None);
    }
}
