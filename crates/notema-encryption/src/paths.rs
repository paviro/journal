use crate::{EncryptionError, Result};
use std::path::{Path, PathBuf};

/// The file locations of a store's key material — everything the encryption
/// layer reads or writes, and nothing about the journal's entries. Public key
/// material lives in the synced `<root>/.age/` folder; the private identity and
/// the roster trust pins live next to the config and are never synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPaths {
    /// The hidden, synced key folder holding the signed `devices.toml` roster and
    /// any `pending-<id>.toml` join requests.
    pub age_dir: PathBuf,
    /// The signed, append-only device roster (`<root>/.age/devices.toml`).
    pub devices_file: PathBuf,
    /// This device's private key material (`identity.toml`), never synced.
    pub identity_file: PathBuf,
    /// This device's local trust pins for the roster (genesis + last-seen head).
    /// Sits next to the identity, never synced, so a sync-folder attacker can't
    /// reach it.
    pub trust_file: PathBuf,
}

impl KeyPaths {
    /// Derive the key locations from the journal root and the config directory.
    pub fn new(journal_root: impl AsRef<Path>, config_dir: impl AsRef<Path>) -> Self {
        let age_dir = journal_root.as_ref().join(".age");
        let config_dir = config_dir.as_ref();
        Self {
            devices_file: age_dir.join("devices.toml"),
            identity_file: config_dir.join("identity.toml"),
            trust_file: config_dir.join("devices-trust.toml"),
            age_dir,
        }
    }

    /// Like [`new`](Self::new), taking the config *file* and reading its parent
    /// directory for the identity location.
    pub fn for_config(config_path: &Path, journal_root: &Path) -> Result<Self> {
        let config_dir = config_path
            .parent()
            .ok_or(EncryptionError::MissingConfigParent)?;
        Ok(Self::new(journal_root, config_dir))
    }

    /// Whether the signed device roster exists — i.e. the store is encrypted.
    pub fn has_roster(&self) -> bool {
        self.devices_file.exists()
    }

    /// Whether this device has generated its private identity here.
    pub fn has_identity(&self) -> bool {
        self.identity_file.exists()
    }
}
