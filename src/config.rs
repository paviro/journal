use crate::AppResult;
use anyhow::{Context, bail};
use notema_storage::{CachePolicy, JournalStore, LibraryDiscovery, LibraryLoadProgress};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

const CONFIG_SCHEMA_VERSION: u32 = 1;
const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) schema_version: u32,
    pub journal: JournalSection,
    #[serde(default)]
    pub attachments: AttachmentsSection,
    #[serde(default)]
    pub ui: UiSection,
    #[serde(default)]
    pub editor: EditorSection,
}

/// Which journals to open and where they live on disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalSection {
    /// Directory holding every journal. A relative path resolves against the
    /// directory containing `config.toml`, so a config dir can carry its
    /// journal root with it.
    pub path: PathBuf,
    /// Journal selected on startup when the previous session didn't record one.
    #[serde(default)]
    pub default: Option<String>,
}

/// How entry attachments are handled.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachmentsSection {
    /// Fetch images referenced by remote URLs into local attachments on import.
    #[serde(default = "default_true")]
    pub download_remote_images: bool,
}

/// Editor behaviour.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditorSection {
    /// Open the entry editor in fullscreen (hiding the other columns) instead
    /// of in-pane.
    #[serde(default)]
    pub start_fullscreen: bool,
}

/// TUI presentation preferences.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct UiSection {
    /// Name of the theme file (without `.toml`) in the config directory's
    /// `themes/` folder. The bundled themes are materialized there on launch.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Whether `{ dark, light }` theme colors follow the detected terminal
    /// background, or are pinned to one variant.
    #[serde(default)]
    pub color_mode: ColorMode,
    /// Chrome style: `default` uses each theme's own `chrome.default_style`; `flat`
    /// or `bordered` force that chrome on every theme.
    #[serde(default)]
    pub chrome: ChromeMode,
    /// Per-device escape hatch: when true, this device ignores the per-journal
    /// themes set in `.journal.toml` sidecars and always uses `theme`. For
    /// low-capability terminals (e-ink) where most themes don't render well.
    #[serde(default)]
    pub ignore_journal_themes: bool,
    #[serde(default)]
    pub layout: LayoutSection,
}

impl Default for UiSection {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            color_mode: ColorMode::default(),
            chrome: ChromeMode::default(),
            ignore_journal_themes: false,
            layout: LayoutSection::default(),
        }
    }
}

fn default_theme() -> String {
    crate::tui::theme::DEFAULT_THEME.to_string()
}

/// Which variant of a theme's `{ dark, light }` colors to use. `Auto` asks the
/// terminal for its background color at startup and falls back to dark.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ColorMode {
    #[default]
    Auto,
    Dark,
    Light,
}

impl ColorMode {
    /// Parse the journal-sidecar spelling; `None` for anything unrecognized (a
    /// newer device's value), so callers fall back to this device's setting.
    /// Must mirror the serde `rename_all = "lowercase"` names.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "auto" => Some(ColorMode::Auto),
            "dark" => Some(ColorMode::Dark),
            "light" => Some(ColorMode::Light),
            _ => None,
        }
    }

    /// The journal-sidecar spelling, inverse of [`Self::from_name`].
    pub(crate) fn name(self) -> &'static str {
        match self {
            ColorMode::Auto => "auto",
            ColorMode::Dark => "dark",
            ColorMode::Light => "light",
        }
    }
}

/// The `[ui] chrome` setting: `default` follows the active theme's
/// `chrome.default_style`; `flat`/`bordered` force that chrome on every theme.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ChromeMode {
    #[default]
    Default,
    Flat,
    Bordered,
}

impl ChromeMode {
    /// The forced [`ChromeStyle`], or `None` for `Default`.
    pub(crate) fn forced_style(self) -> Option<crate::tui::theme::ChromeStyle> {
        match self {
            ChromeMode::Default => None,
            ChromeMode::Flat => Some(crate::tui::theme::ChromeStyle::Flat),
            ChromeMode::Bordered => Some(crate::tui::theme::ChromeStyle::Bordered),
        }
    }

    /// Inverse of [`Self::forced_style`].
    pub(crate) fn from_override(style: Option<crate::tui::theme::ChromeStyle>) -> Self {
        match style {
            None => ChromeMode::Default,
            Some(crate::tui::theme::ChromeStyle::Flat) => ChromeMode::Flat,
            Some(crate::tui::theme::ChromeStyle::Bordered) => ChromeMode::Bordered,
        }
    }

    /// Parse the journal-sidecar spelling; `None` for anything unrecognized (a
    /// newer device's value), so callers fall back to this device's setting.
    /// Must mirror the serde `rename_all = "lowercase"` names.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "default" => Some(ChromeMode::Default),
            "flat" => Some(ChromeMode::Flat),
            "bordered" => Some(ChromeMode::Bordered),
            _ => None,
        }
    }

    /// The journal-sidecar spelling, inverse of [`Self::from_name`].
    pub(crate) fn name(self) -> &'static str {
        match self {
            ChromeMode::Default => "default",
            ChromeMode::Flat => "flat",
            ChromeMode::Bordered => "bordered",
        }
    }
}

/// Layout geometry: how the panels and their contents are sized. Column-width and
/// breakpoint knobs would live directly here; reader settings sit in their
/// own sub-table to keep the two concerns separate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayoutSection {
    #[serde(default)]
    pub reader: ReaderSection,
}

/// How the reader pane presents an entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReaderSection {
    /// Vertically center the body in the reader when it fits without scrolling.
    #[serde(default = "default_true")]
    pub body_center_vertically: bool,
    /// Max width, in cells, of the entry body; wider panels gutter the sides so
    /// long-form text stays readable. Metadata keeps the full width.
    #[serde(default = "default_body_max_width")]
    pub body_max_width: u16,
    /// Show each link's target URL as a faint `(url)` after its name. Off by
    /// default — the name is clickable either way, so the URL is just noise.
    #[serde(default)]
    pub show_link_urls: bool,
}

impl Default for AttachmentsSection {
    fn default() -> Self {
        Self {
            download_remote_images: true,
        }
    }
}

impl Default for ReaderSection {
    fn default() -> Self {
        Self {
            body_center_vertically: true,
            body_max_width: default_body_max_width(),
            show_link_urls: false,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_body_max_width() -> u16 {
    100
}

impl Config {
    pub(crate) fn new(journal_root: PathBuf) -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            journal: JournalSection {
                path: expand_tilde(journal_root),
                default: None,
            },
            attachments: AttachmentsSection::default(),
            ui: UiSection::default(),
            editor: EditorSection::default(),
        }
    }
}

/// Per-device, machine-written UI state kept next to `config.toml` in `state.toml`
/// (never synced). Separated from [`Config`] so the user's hand-edited settings
/// stay free of values the app rewrites on its own — and so the file has room to
/// grow as more session state is remembered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    pub(crate) schema_version: u32,
    /// The stable id of the journal selected when the TUI last exited, restored on
    /// next launch. Stored by id (not name) so it survives a rename or archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_journal_id: Option<String>,
    #[serde(default)]
    pub ui: UiState,
}

impl Default for State {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            last_journal_id: None,
            ui: UiState::default(),
        }
    }
}

/// Toggle states for optional TUI chrome, flipped by keybindings and remembered
/// across launches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct UiState {
    /// Whether the footer keybinding hints are shown.
    #[serde(default = "default_true")]
    pub show_hints: bool,
    /// Whether the journals panel is shown.
    #[serde(default = "default_true")]
    pub show_journals: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            show_hints: true,
            show_journals: true,
        }
    }
}

pub(crate) fn default_config_path() -> AppResult<PathBuf> {
    // An explicit XDG_CONFIG_HOME always wins, on every platform.
    if let Ok(config_home) = env::var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home)
            .join("notema")
            .join("config.toml"));
    }

    // macOS keeps app data under Application Support; other Unixes use ~/.config,
    // where the app name is already the namespace.
    #[cfg(target_os = "macos")]
    let dir = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/de.paviro.notema"))
        .context("HOME is not set")?;
    #[cfg(not(target_os = "macos"))]
    let dir = dirs::home_dir()
        .context("could not determine home directory")?
        .join(".config")
        .join("notema");
    Ok(dir.join("config.toml"))
}

pub(crate) fn load_config(path: &Path) -> AppResult<Config> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading config {}", path.display()))?;
    let mut config: Config =
        toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
    if config.schema_version != CONFIG_SCHEMA_VERSION {
        bail!(
            "unsupported config schema version {}; expected {CONFIG_SCHEMA_VERSION}",
            config.schema_version
        );
    }
    config.journal.path = resolve_journal_root(config.journal.path, path);
    Ok(config)
}

/// Expand `~` and anchor a relative journal root to the config file's
/// directory, so a config dir can reference a journal root beside itself
/// regardless of the process's working directory.
fn resolve_journal_root(root: PathBuf, config_path: &Path) -> PathBuf {
    let root = expand_tilde(root);
    match config_path.parent() {
        Some(dir) if root.is_relative() => dir.join(root),
        _ => root,
    }
}

pub(crate) fn save_config(path: &Path, config: &Config) -> AppResult<()> {
    write_toml_atomic(path, &toml::to_string_pretty(config)?)
}

/// The device's `state.toml`, kept beside `config.toml` in the same directory.
pub(crate) fn state_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("state.toml")
}

/// Load this device's UI state, defaulting when `state.toml` doesn't exist yet.
pub(crate) fn load_state(config_path: &Path) -> AppResult<State> {
    let path = state_path(config_path);
    if !path.exists() {
        return Ok(State::default());
    }
    let text = fs::read_to_string(&path)?;
    match toml::from_str::<State>(&text) {
        Ok(state) if state.schema_version == STATE_SCHEMA_VERSION => Ok(state),
        Ok(state) => reset_invalid_state(
            &path,
            format!("unsupported schema version {}", state.schema_version),
        ),
        Err(error) => reset_invalid_state(&path, error.to_string()),
    }
}

fn reset_invalid_state(path: &Path, reason: String) -> AppResult<State> {
    let backup = path.with_extension("toml.invalid");
    fs::rename(path, &backup)?;
    eprintln!(
        "Ignored invalid UI state ({reason}); backup saved to {}",
        backup.display()
    );
    Ok(State::default())
}

pub(crate) fn save_state(config_path: &Path, state: &State) -> AppResult<()> {
    write_toml_atomic(&state_path(config_path), &toml::to_string_pretty(state)?)
}

/// Write `text` to `path` atomically: a same-directory temp file plus a rename, so
/// a crash mid-write leaves the previous file intact rather than a truncated one.
/// Also used by the theme engine to materialize the bundled theme files. Backed by
/// the shared write-fsync-rename-fsync primitive, whose temp names are unique per
/// process so two notema instances saving state can't clobber each other's temp.
pub(crate) fn write_toml_atomic(path: &Path, text: &str) -> AppResult<()> {
    Ok(notema_encryption::atomic_write(path, text.as_bytes())?)
}

/// What `load_or_setup_with_path` resolved: a store ready to open in the TUI. An
/// encrypted store this device can't yet read is opened too — the TUI shows the
/// enroll/awaiting notice rather than the CLI printing it.
pub(crate) struct Startup {
    pub config_path: PathBuf,
    pub config: Config,
    pub store: JournalStore,
    pub discovery: Option<LibraryDiscovery>,
}

pub(crate) fn load_or_setup_with_path(path_override: Option<&Path>) -> AppResult<Startup> {
    let config_path = config_path(path_override)?;

    // An encrypted store this device can't yet read (no key, awaiting approval, or
    // revoked) is still opened: the TUI shows the enroll/awaiting notice instead
    // of the CLI printing a hint, so every unreadable-store case looks the same.
    // Reconciling a remote encryption *disable* is likewise deferred to the TUI,
    // which must run it before probing for a lock.
    let (config, store, discovery) = if config_path.exists() {
        let config = load_config(&config_path)?;
        let prepared = crate::ish::prepare_store(&config_path, &config.journal.path, true)?;
        (config, prepared.store, prepared.discovery)
    } else {
        let (config, store) = interactive_setup(&config_path)?;
        (config, store, None)
    };

    Ok(Startup {
        config_path,
        config,
        store,
        discovery,
    })
}

pub(crate) fn load_existing(path_override: Option<&Path>) -> AppResult<Startup> {
    let config_path = config_path(path_override)?;
    if !config_path.exists() {
        bail!(
            "config file not found at {}; run `journal` once to set it up or pass --config <DIR>",
            config_path.display()
        );
    }

    let config = load_config(&config_path)?;
    let store = crate::ish::prepare_store(&config_path, &config.journal.path, false)?.store;
    if store.reconcile_disabled_encryption()? {
        eprintln!(
            "Note: encryption was disabled on another device; retired this device's key and trust pins."
        );
    }
    Ok(Startup {
        config_path,
        config,
        store,
        discovery: None,
    })
}

/// Resolve the config *file* from an optional config-directory override. The
/// override names the directory that holds `config.toml` alongside this device's
/// encryption key; without one we fall back to the XDG default.
fn config_path(path_override: Option<&Path>) -> AppResult<PathBuf> {
    match path_override {
        Some(dir) => {
            // `--config` names the directory, not the file. Passing a file (a
            // stale `.../config.toml`) would silently nest into
            // `.../config.toml/config.toml` and trigger a bogus first-run setup.
            if dir.is_file() || dir.file_name() == Some(std::ffi::OsStr::new("config.toml")) {
                bail!(
                    "--config takes a directory, not a file; pass {} instead",
                    dir.parent().unwrap_or(dir).display()
                );
            }
            Ok(dir.join("config.toml"))
        }
        None => default_config_path(),
    }
}

fn interactive_setup(config_path: &Path) -> AppResult<(Config, JournalStore)> {
    let mut stdout = io::stdout();
    let default_root = dirs::home_dir()
        .map(|home| home.join("Journals"))
        .unwrap_or_else(|| PathBuf::from("Journals"));

    writeln!(stdout, "Notema first-run setup")?;

    // On iSH the journal lives on a mounted iOS folder, so there's no path to
    // type: use a fixed mountpoint and let the iOS picker choose the folder.
    let journal_root = if crate::ish::is_ish() {
        let mountpoint = PathBuf::from(crate::ish::DEFAULT_MOUNTPOINT);
        crate::ish::ensure_journal_mounted(&mountpoint)?;
        mountpoint
    } else {
        write!(
            stdout,
            "Journal root [{}]: ",
            default_root.to_string_lossy()
        )?;
        stdout.flush()?;

        let mut root_input = String::new();
        io::stdin().read_line(&mut root_input)?;
        if root_input.trim().is_empty() {
            default_root
        } else {
            PathBuf::from(root_input.trim())
        }
    };

    let mut config = Config::new(journal_root);

    // E-ink and other limited-palette displays get `classic`, which renders on
    // terminals without true-color; everything else starts on the default theme.
    write!(stdout, "Is this an e-ink / monochrome display? [y/N]: ")?;
    stdout.flush()?;
    let mut eink_input = String::new();
    io::stdin().read_line(&mut eink_input)?;
    if is_yes(&eink_input) {
        config.ui.theme = "classic".to_string();
    }
    let prepared = crate::ish::prepare_store(config_path, &config.journal.path, true)?;
    let store = prepared.store;

    if should_offer_encryption(&store)? {
        offer_encryption(&mut stdout, &store)?;
    } else if !store.encryption_enabled() {
        // An existing plaintext journal is registered as-is; encryption stays a
        // deliberate later step rather than a first-run prompt.
        writeln!(
            stdout,
            "Using existing journal at {}. Encryption is off; run `notema encryption enable` to turn it on.",
            config.journal.path.display()
        )?;
    }

    if crate::ish::is_ish() {
        writeln!(
            stdout,
            "Note: iSH does not support live file watching. Notema refreshes automatically on each startup; new entries may take a little time to appear while that refresh runs. Press r to refresh immediately."
        )?;
    }

    save_config(config_path, &config)?;
    if !store.encryption_enabled() {
        let progress = Mutex::new(SetupCacheProgress::new(&mut stdout, crate::ish::is_ish()));
        let update = |update| {
            progress
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .update(update);
        };
        let snapshot = match prepared.discovery {
            Some(discovery) => store.load_discovered_library_with_progress(
                CachePolicy::Rebuild,
                discovery,
                &update,
            ),
            None => store.load_library_with_progress(CachePolicy::Rebuild, &update),
        };
        let progress_result = progress
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finish();
        progress_result?;
        let snapshot = snapshot?;
        if let Some(warning) = snapshot.report.cache_warning {
            writeln!(stdout, "Warning: {warning}")?;
        }
    }
    Ok((config, store))
}

struct SetupCacheProgress<'a, W: Write> {
    writer: &'a mut W,
    ish: bool,
    discovery_started: bool,
    reading_started: bool,
    discovered: usize,
    last_discovered: usize,
    last_read: usize,
    error: Option<io::Error>,
}

impl<'a, W: Write> SetupCacheProgress<'a, W> {
    const WIDTH: usize = 20;

    fn new(writer: &'a mut W, ish: bool) -> Self {
        Self {
            writer,
            ish,
            discovery_started: false,
            reading_started: false,
            discovered: 0,
            last_discovered: 0,
            last_read: 0,
            error: None,
        }
    }

    fn update(&mut self, update: LibraryLoadProgress) {
        if self.error.is_some() {
            return;
        }

        let result = match update {
            LibraryLoadProgress::Discovering { entries_found } => {
                self.update_discovery(entries_found)
            }
            LibraryLoadProgress::Reading { current, total } => self.update_reading(current, total),
        };
        if let Err(error) = result {
            self.error = Some(error);
        }
    }

    fn update_discovery(&mut self, entries_found: usize) -> io::Result<()> {
        self.discovered = entries_found;
        if !self.discovery_started {
            if self.ish {
                writeln!(self.writer, "First setup: scanning journal files.")?;
                writeln!(
                    self.writer,
                    "This can take a long time on iSH; later starts are fast."
                )?;
            }
            self.discovery_started = true;
        }
        if entries_found != 0 && entries_found.saturating_sub(self.last_discovered) < 25 {
            return Ok(());
        }
        self.last_discovered = entries_found;
        write!(
            self.writer,
            "\rScanning journal files… {entries_found} entries found"
        )?;
        self.writer.flush()
    }

    fn update_reading(&mut self, current: usize, total: usize) -> io::Result<()> {
        if !self.reading_started {
            if self.discovery_started {
                if self.discovered != self.last_discovered {
                    self.last_discovered = self.discovered;
                    write!(
                        self.writer,
                        "\rScanning journal files… {} entries found",
                        self.discovered
                    )?;
                }
                writeln!(self.writer)?;
            }
            self.reading_started = true;
        }
        if current < self.last_read {
            return Ok(());
        }
        let step = (total / 100).max(1);
        if current != 0 && current != total && current.saturating_sub(self.last_read) < step {
            return Ok(());
        }
        self.last_read = current;
        let filled = current
            .min(total)
            .saturating_mul(Self::WIDTH)
            .checked_div(total)
            .unwrap_or(Self::WIDTH);
        let bar = format!("{}{}", "#".repeat(filled), "-".repeat(Self::WIDTH - filled));
        write!(
            self.writer,
            "\rIndexing journal entries [{bar}] {current}/{total}"
        )?;
        self.writer.flush()
    }

    fn finish(mut self) -> io::Result<()> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        writeln!(self.writer)
    }
}

/// First-run offers to enable encryption only for a brand-new, empty root — never
/// for a journal that already has entries or is already encrypted. Those are just
/// registered; encryption is managed with the `notema encryption …` commands.
fn should_offer_encryption(store: &JournalStore) -> AppResult<bool> {
    Ok(!store.encryption_enabled() && store.list_journals()?.is_empty())
}

/// Prompt to enable encryption on a fresh store and, if accepted, generate this
/// device's identity. Holds for a keypress afterward because the TUI's alternate
/// screen would otherwise wipe the identity-backup warning.
fn offer_encryption(stdout: &mut impl Write, store: &JournalStore) -> AppResult<()> {
    write!(stdout, "Enable encryption? [y/N]: ")?;
    stdout.flush()?;
    let mut encryption_input = String::new();
    io::stdin().read_line(&mut encryption_input)?;
    if !is_yes(&encryption_input) {
        return Ok(());
    }

    let (device_name, passphrase) = crate::prompts::resolve_new_identity_options(None, false)?;
    store.initialize_encryption(&device_name, passphrase.as_ref())?;
    writeln!(
        stdout,
        "Identity file: {}. Back it up; without it encrypted journal files cannot be decrypted.",
        store.identity_path().display()
    )?;
    if passphrase.is_none() {
        writeln!(
            stdout,
            "This key has no passphrase, so anyone with this file can read the journal — keep the device and its backups secure."
        )?;
    }

    write!(stdout, "\nPress Enter to open your journal…")?;
    stdout.flush()?;
    let mut ack = String::new();
    io::stdin().read_line(&mut ack)?;
    Ok(())
}

/// Interpret an interactive `[y/N]` answer, defaulting to no.
fn is_yes(input: &str) -> bool {
    matches!(input.trim(), "y" | "Y" | "yes" | "YES" | "Yes")
}

pub(crate) fn expand_tilde(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return dirs::home_dir().unwrap_or(path);
    }

    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }

    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store_in(dir: &Path) -> JournalStore {
        let store =
            JournalStore::for_config(&dir.join("config.toml"), &dir.join("journals")).unwrap();
        store.ensure().unwrap();
        store
    }

    #[test]
    fn mode_and_chrome_names_round_trip_and_match_serde() {
        for mode in [ColorMode::Auto, ColorMode::Dark, ColorMode::Light] {
            assert_eq!(ColorMode::from_name(mode.name()), Some(mode));
            // The sidecar spelling must stay the config-file spelling.
            assert_eq!(
                toml::Value::try_from(mode).unwrap().as_str(),
                Some(mode.name())
            );
        }
        for chrome in [ChromeMode::Default, ChromeMode::Flat, ChromeMode::Bordered] {
            assert_eq!(ChromeMode::from_name(chrome.name()), Some(chrome));
            assert_eq!(
                toml::Value::try_from(chrome).unwrap().as_str(),
                Some(chrome.name())
            );
            assert_eq!(ChromeMode::from_override(chrome.forced_style()), chrome);
        }
        assert_eq!(ColorMode::from_name("neon"), None);
        assert_eq!(ChromeMode::from_name("neon"), None);
    }

    #[test]
    fn offers_encryption_only_for_an_empty_new_root() {
        let dir = tempdir().unwrap();
        let store = store_in(dir.path());
        assert!(should_offer_encryption(&store).unwrap());
    }

    #[test]
    fn skips_encryption_prompt_for_an_existing_plaintext_journal() {
        let dir = tempdir().unwrap();
        let store = store_in(dir.path());
        store.create_journal("work").unwrap();
        assert!(!should_offer_encryption(&store).unwrap());
    }

    #[test]
    fn skips_encryption_prompt_for_an_already_encrypted_store() {
        let dir = tempdir().unwrap();
        let store = store_in(dir.path());
        store.initialize_encryption("laptop", None).unwrap();
        assert!(!should_offer_encryption(&store).unwrap());
    }

    #[test]
    fn config_path_rejects_a_file_argument() {
        let err = config_path(Some(Path::new("/some/dir/config.toml"))).unwrap_err();
        assert!(err.to_string().contains("takes a directory"));
    }

    #[test]
    fn save_and_load_config_expands_tilde_root() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "schema_version = 1\n\n[journal]\npath = \"~/Journals\"\n",
        )
        .unwrap();

        let config = load_config(&path).unwrap();

        assert!(config.journal.path.ends_with("Journals"));
        assert_eq!(config.journal.default, None);
    }

    #[test]
    fn load_config_resolves_relative_root_against_config_dir() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "schema_version = 1\n\n[journal]\npath = \"journals\"\n").unwrap();

        let config = load_config(&path).unwrap();

        assert_eq!(config.journal.path, dir.path().join("journals"));
    }

    #[test]
    fn save_and_load_config_round_trips_all_fields() {
        let dir = tempdir().unwrap();
        // A nested path also exercises that save creates missing parent dirs.
        let path = dir.path().join("nested").join("config.toml");
        let mut config = Config::new(dir.path().join("root"));
        config.journal.default = Some("work".to_string());
        config.attachments.download_remote_images = false;
        config.ui.theme = "eclipse".to_string();
        config.ui.color_mode = ColorMode::Light;
        config.ui.layout.reader.body_center_vertically = false;
        config.ui.layout.reader.body_max_width = 80;

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn missing_optional_fields_use_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "schema_version = 1\n\n[journal]\npath = \"~/Journals\"\n",
        )
        .unwrap();

        let config = load_config(&path).unwrap();

        assert!(config.attachments.download_remote_images);
        assert_eq!(config.ui.theme, crate::tui::theme::DEFAULT_THEME);
        assert_eq!(config.ui.color_mode, ColorMode::Auto);
        assert!(config.ui.layout.reader.body_center_vertically);
        assert_eq!(config.ui.layout.reader.body_max_width, 100);
    }

    #[test]
    fn save_and_load_state_round_trips_and_defaults_when_missing() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        // Missing state.toml loads as the default: no journal remembered, chrome shown.
        let default = load_state(&config_path).unwrap();
        assert_eq!(default, State::default());
        assert!(default.ui.show_hints);
        assert!(default.ui.show_journals);

        let state = State {
            schema_version: STATE_SCHEMA_VERSION,
            last_journal_id: Some("home1234".to_string()),
            ui: UiState {
                show_hints: false,
                show_journals: true,
            },
        };
        save_state(&config_path, &state).unwrap();

        assert_eq!(state_path(&config_path), dir.path().join("state.toml"));
        assert_eq!(load_state(&config_path).unwrap(), state);
    }

    #[test]
    fn first_start_cache_progress_reaches_a_full_bar() {
        let mut output = Vec::new();
        let mut progress = SetupCacheProgress::new(&mut output, true);

        progress.update(LibraryLoadProgress::Discovering { entries_found: 0 });
        progress.update(LibraryLoadProgress::Discovering { entries_found: 4 });
        progress.update(LibraryLoadProgress::Reading {
            current: 0,
            total: 4,
        });
        progress.update(LibraryLoadProgress::Reading {
            current: 2,
            total: 4,
        });
        progress.update(LibraryLoadProgress::Reading {
            current: 4,
            total: 4,
        });
        progress.finish().unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("First setup: scanning journal files."));
        assert!(output.contains("later starts are fast"));
        assert!(output.contains("Scanning journal files… 0 entries found"));
        assert!(output.contains("[##########----------] 2/4"));
        assert!(output.contains("[####################] 4/4"));
        assert!(output.ends_with('\n'));
    }
}
