use crate::AppResult;
use anyhow::{Context, bail};
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, fs, path::Path, str::FromStr};

const STORE_MARKER_FILE: &str = ".notema-store.toml";
const STORE_MARKER_SCHEMA_VERSION: u32 = 1;
const STORE_ID_BYTES: usize = 16;
const STORE_ID_HEX_LEN: usize = STORE_ID_BYTES * 2;

/// Stable identity for one synced journal root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct StoreId(String);

impl StoreId {
    pub(crate) fn generate() -> AppResult<Self> {
        let mut bytes = [0_u8; STORE_ID_BYTES];
        getrandom::fill(&mut bytes).context("generating journal store id")?;
        Ok(Self(hex::encode(bytes)))
    }
}

impl fmt::Display for StoreId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for StoreId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != STORE_ID_HEX_LEN
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("journal store id must be 32 lowercase hexadecimal characters");
        }
        Ok(Self(value.to_owned()))
    }
}

impl<'de> Deserialize<'de> for StoreId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreMarker {
    schema_version: u32,
    store_id: StoreId,
}

pub(crate) fn marker_path(root: &Path) -> std::path::PathBuf {
    root.join(STORE_MARKER_FILE)
}

pub(crate) fn read(root: &Path) -> AppResult<Option<StoreId>> {
    let path = marker_path(root);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let marker: StoreMarker =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if marker.schema_version != STORE_MARKER_SCHEMA_VERSION {
        bail!(
            "unsupported journal store marker schema {} in {}",
            marker.schema_version,
            path.display()
        );
    }
    Ok(Some(marker.store_id))
}

pub(crate) fn ensure(root: &Path) -> AppResult<StoreId> {
    if let Some(store_id) = read(root)? {
        return Ok(store_id);
    }
    let marker = StoreMarker {
        schema_version: STORE_MARKER_SCHEMA_VERSION,
        store_id: StoreId::generate()?,
    };
    let encoded = toml::to_string_pretty(&marker)?;
    notema_encryption::atomic_write(&marker_path(root), encoded.as_bytes())?;
    Ok(marker.store_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_marker_is_created_once_and_stays_stable() {
        let dir = tempdir().unwrap();

        let first = ensure(dir.path()).unwrap();
        let second = ensure(dir.path()).unwrap();

        assert_eq!(first, second);
        assert_eq!(read(dir.path()).unwrap(), Some(first));
    }

    #[test]
    fn corrupt_and_future_markers_are_rejected_without_replacement() {
        let dir = tempdir().unwrap();
        let path = marker_path(dir.path());
        fs::write(
            &path,
            "schema_version = 2\nstore_id = \"00000000000000000000000000000000\"\n",
        )
        .unwrap();
        let original = fs::read(&path).unwrap();

        assert!(ensure(dir.path()).is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }
}
