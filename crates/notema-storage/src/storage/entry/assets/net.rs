//! Fetching external image sources: local file reads and gated `http(s)`
//! downloads, with a per-run host-reachability probe so a dead host referenced
//! by many links is only waited on once.

use super::{expand_user, image_extension, is_url, url_path};
use std::{
    collections::HashMap,
    fs,
    net::{TcpStream, ToSocketAddrs},
    sync::{Mutex, OnceLock, mpsc},
    thread,
    time::Duration,
};

/// Upper bound on a downloaded image (bytes).
const MAX_REMOTE_IMAGE_BYTES: u64 = 50 * 1024 * 1024;
/// Network timeout for opt-in remote image ingestion.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for the cheap "is this host up?" probe done before a full download.
const HOST_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Categorized fetch failure so callers can build the right report entry without
/// parsing message text. `RemoteUnavailable` is benign (downloads off or host
/// down); `Ingest` is a genuine failure worth surfacing.
pub(super) enum FetchError {
    RemoteUnavailable,
    Ingest(String),
}

/// Read a local file or download a URL, returning its bytes and image extension.
pub(super) fn fetch_source(
    source: &str,
    download_remote: bool,
) -> Result<(Vec<u8>, String), FetchError> {
    if is_url(source) {
        if !download_remote {
            return Err(FetchError::RemoteUnavailable);
        }
        let bytes = download(source)?;
        let ext = image_extension(url_path(source), &bytes)
            .ok_or_else(|| FetchError::Ingest("not a supported image".to_string()))?;
        Ok((bytes, ext))
    } else {
        let path = expand_user(source);
        let bytes = fs::read(&path).map_err(|error| FetchError::Ingest(error.to_string()))?;
        let ext = image_extension(source, &bytes)
            .ok_or_else(|| FetchError::Ingest("not a supported image".to_string()))?;
        Ok((bytes, ext))
    }
}

fn download(url: &str) -> Result<Vec<u8>, FetchError> {
    // Probe the host first (once per host, cached). A bulk import can reference
    // hundreds of links on a server that no longer exists; without this each
    // one would block for the full `REMOTE_TIMEOUT` before failing.
    if let Some((host, port)) = host_port(url)
        && !host_reachable(&host, port)
    {
        return Err(FetchError::RemoteUnavailable);
    }

    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REMOTE_TIMEOUT))
        .build();
    let agent: ureq::Agent = config.into();
    let bytes = agent
        .get(url)
        .call()
        .map_err(|error| FetchError::Ingest(error.to_string()))?
        .body_mut()
        .with_config()
        .limit(MAX_REMOTE_IMAGE_BYTES)
        .read_to_vec()
        .map_err(|error| FetchError::Ingest(error.to_string()))?;
    Ok(bytes)
}

/// Per-process cache of host reachability (`"host:port" -> up?`), so a dead host
/// referenced by many links is probed only once for the lifetime of the run.
fn host_status_cache() -> &'static Mutex<HashMap<String, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn host_reachable(host: &str, port: u16) -> bool {
    let key = format!("{host}:{port}");
    if let Some(&reachable) = host_status_cache().lock().unwrap().get(&key) {
        return reachable;
    }
    let reachable = probe_host(host, port);
    host_status_cache().lock().unwrap().insert(key, reachable);
    reachable
}

/// Attempt a TCP connection to `host:port`, bounding both DNS resolution and the
/// connect by `HOST_PROBE_TIMEOUT`. Resolution has no native timeout, so it runs
/// on a helper thread we stop waiting on once the deadline passes.
fn probe_host(host: &str, port: u16) -> bool {
    let host = host.to_string();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let reachable = (host.as_str(), port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
            .map(|addr| TcpStream::connect_timeout(&addr, HOST_PROBE_TIMEOUT).is_ok())
            .unwrap_or(false);
        let _ = tx.send(reachable);
    });
    rx.recv_timeout(HOST_PROBE_TIMEOUT).unwrap_or(false)
}

/// Extract `(host, port)` from an `http(s)` URL, defaulting the port by scheme.
/// Returns `None` for unparseable authorities (e.g. bracketed IPv6), in which
/// case the caller downloads without a pre-probe.
fn host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // Drop any `user:pass@` prefix.
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    if host_port.contains('[') {
        return None; // Skip IPv6 literals rather than mis-parse them.
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => (host, port.parse().ok()?),
        _ => {
            let default = if scheme.eq_ignore_ascii_case("https") {
                443
            } else {
                80
            };
            (host_port, default)
        }
    };
    (!host.is_empty()).then(|| (host.to_string(), port))
}
