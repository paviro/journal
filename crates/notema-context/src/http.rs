//! A tiny blocking HTTP GET shared by the keyless network lookups (geocoding via
//! Nominatim, weather via Open-Meteo). One `ureq` agent per call, a global
//! timeout, and a hard cap on the response body — enough for the small JSON
//! payloads these APIs return.

use crate::Result;
use std::{sync::OnceLock, time::Duration};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound on a response body (bytes) — the JSON these APIs return is tiny.
pub(crate) const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;
/// Identifies the application. Nominatim's policy requires a descriptive
/// `User-Agent` (a stock HTTP library one is rejected); Open-Meteo doesn't care,
/// so one value serves both.
pub(crate) const USER_AGENT: &str = concat!("notema-tui/", env!("CARGO_PKG_VERSION"));

/// Fetch `url` as a UTF-8 string, or an error on transport/HTTP/decoding failure.
pub(crate) fn get(url: &str) -> Result<String> {
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

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build()
            .into()
    })
}
