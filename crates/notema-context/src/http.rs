//! A tiny blocking HTTP GET shared by the keyless network lookups (geocoding via
//! Nominatim, weather via Open-Meteo). One `ureq` agent per call, a global
//! timeout, and a hard cap on the response body — enough for the small JSON
//! payloads these APIs return.

use crate::Result;
use std::{path::Path, sync::OnceLock, sync::mpsc, thread, time::Duration};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(10);
/// Cap on establishing the TCP connection. Shorter than the global timeout so a
/// dead or black-holed network (offline, dropped SYNs, captive portals) gives up
/// fast, while a slow-but-alive server still gets the full [`TIMEOUT`] to respond.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Upper bound on a response body (bytes) — the JSON these APIs return is tiny.
pub(crate) const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;
/// Identifies the application. Nominatim's policy requires a descriptive
/// `User-Agent` (a stock HTTP library one is rejected); Open-Meteo doesn't care,
/// so one value serves both.
pub(crate) const USER_AGENT: &str = concat!("notema-tui/", env!("CARGO_PKG_VERSION"));

/// Fetch `url` as a UTF-8 string, or an error on transport/HTTP/decoding failure.
pub(crate) fn get(url: &str) -> Result<String> {
    if is_ish() {
        return get_with_user_space_timeout(url);
    }
    get_inner(url)
}

fn get_inner(url: &str) -> Result<String> {
    let body = agent()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()?
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()?;
    Ok(body)
}

fn get_with_user_space_timeout(url: &str) -> Result<String> {
    let url = url.to_string();
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = tx.send(get_inner(&url));
    });
    match rx.recv_timeout(TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(crate::ContextError::message(format!(
            "context provider request timed out after {} seconds",
            TIMEOUT.as_secs()
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(crate::ContextError::message(
            "context provider request worker stopped unexpectedly",
        )),
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| agent_config_for(is_ish()).into())
}

fn agent_config_for(ish: bool) -> ureq::config::Config {
    let builder = ureq::Agent::config_builder();
    let builder = if ish {
        builder.no_delay(false)
    } else {
        builder
            .timeout_global(Some(TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
    };
    #[cfg(feature = "tls-native")]
    let builder = builder.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .build(),
    );
    builder.build()
}

fn is_ish() -> bool {
    cfg!(target_os = "linux") && Path::new("/proc/ish/version").exists()
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "tls-native")]
    #[test]
    fn native_tls_feature_selects_native_tls_provider() {
        assert_eq!(
            super::agent_config_for(false).tls_config().provider(),
            ureq::tls::TlsProvider::NativeTls
        );
    }

    #[test]
    fn ish_avoids_kernel_socket_options() {
        let config = super::agent_config_for(true);
        assert!(!config.no_delay());
        assert_eq!(config.timeouts().global, None);
    }

    #[test]
    fn other_platforms_keep_the_global_timeout() {
        let config = super::agent_config_for(false);
        assert!(config.no_delay());
        assert_eq!(config.timeouts().global, Some(super::TIMEOUT));
        // A short connect cap fails fast on a dead network without shortening the
        // read budget for a slow-but-alive server.
        assert_eq!(config.timeouts().connect, Some(super::CONNECT_TIMEOUT));
    }

    #[cfg(all(feature = "tls-ring", not(feature = "tls-native")))]
    #[test]
    fn ring_feature_selects_rustls_provider() {
        assert_eq!(
            super::agent_config_for(false).tls_config().provider(),
            ureq::tls::TlsProvider::Rustls
        );
    }
}
