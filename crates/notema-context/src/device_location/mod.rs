//! Ask the device for its current position ("grab current GPS"), so the location
//! dialog can fill itself in without the user typing coordinates.
//!
//! Each platform has its own provider, gated so nothing is pulled in where it
//! isn't used: Termux via the `termux-location` command, Linux via GeoClue2 over
//! D-Bus, macOS via CoreLocation. A provider only has to produce a `lat/lon`; the
//! dialog then reverse-geocodes it into a named place through the existing
//! [`reverse_geocode`](crate::reverse_geocode) path.

use crate::{ContextError, Result};
use notema_domain::Coordinates;
use serde::Deserialize;
use std::{sync::mpsc, thread, time::Duration};

#[cfg(target_os = "linux")]
mod geoclue;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "android")]
mod termux;

/// Which backend produced a fix — recorded on the saved location's metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceLocationSource {
    /// macOS CoreLocation.
    CoreLocation,
    /// Linux GeoClue2.
    GeoClue,
    /// Termux `termux-location` (Android).
    Termux,
}

impl DeviceLocationSource {
    /// The slug stored in an entry's location metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CoreLocation => "corelocation",
            Self::GeoClue => "geoclue",
            Self::Termux => "termux",
        }
    }
}

impl std::fmt::Display for DeviceLocationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single position reading from the device.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceFix {
    pub coordinates: Coordinates,
    /// Horizontal accuracy in metres, when the provider reports it.
    pub accuracy_m: Option<f64>,
    pub source: DeviceLocationSource,
}

/// Get the device's current location. Blocking (GPS fixes take a moment), so it's
/// meant for the geocode worker thread, never the UI thread. Returns a clear,
/// user-facing error when the platform has no provider, the provider is missing,
/// or no fix could be obtained.
pub fn device_location() -> Result<DeviceFix> {
    #[cfg(target_os = "android")]
    {
        termux::locate()
    }
    #[cfg(target_os = "linux")]
    {
        geoclue::locate()
    }
    #[cfg(target_os = "macos")]
    {
        macos::locate()
    }
    #[cfg(not(any(target_os = "android", target_os = "linux", target_os = "macos")))]
    {
        Err(ContextError::message(
            "grabbing the device location isn't supported on this platform",
        ))
    }
}

/// Run `f` on a helper thread and give up after `timeout` (`None` = timed out).
/// The thread is detached on timeout, so a caller that spawned a child process is
/// responsible for killing it.
#[cfg_attr(
    not(any(target_os = "android", target_os = "linux", target_os = "macos")),
    allow(dead_code)
)]
fn run_with_timeout<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(timeout).ok()
}

/// Parse a `{"latitude":..,"longitude":..,"accuracy":..}` fix (as printed by
/// `termux-location` and the macOS `notema-locate` helper) into a [`DeviceFix`].
/// Kept out of the platform modules so it can be unit-tested on any host.
/// `accuracy` is optional; missing coordinates mean no fix was obtained.
#[cfg_attr(not(any(target_os = "android", target_os = "macos")), allow(dead_code))]
fn parse_fix_json(body: &str, source: DeviceLocationSource) -> Result<DeviceFix> {
    #[derive(Deserialize)]
    struct Raw {
        latitude: Option<f64>,
        longitude: Option<f64>,
        accuracy: Option<f64>,
    }
    let raw: Raw = serde_json::from_str(body.trim())
        .map_err(|_| ContextError::message("location helper returned unexpected output"))?;
    match (raw.latitude, raw.longitude) {
        (Some(latitude), Some(longitude)) => Coordinates::try_new(latitude, longitude)
            .map(|coordinates| DeviceFix {
                coordinates,
                accuracy_m: raw
                    .accuracy
                    .filter(|accuracy| accuracy.is_finite() && *accuracy >= 0.0),
                source,
            })
            .map_err(|error| ContextError::message(error.to_string())),
        _ => Err(ContextError::message("no location fix was obtained")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fix_json_reads_coordinates_and_accuracy() {
        let body = r#"{
            "latitude": 52.52,
            "longitude": 13.405,
            "altitude": 34.0,
            "accuracy": 18.5,
            "bearing": 0.0,
            "speed": 0.0,
            "provider": "gps"
        }"#;
        let fix = parse_fix_json(body, DeviceLocationSource::Termux).unwrap();
        assert_eq!(fix.coordinates.latitude(), 52.52);
        assert_eq!(fix.coordinates.longitude(), 13.405);
        assert_eq!(fix.accuracy_m, Some(18.5));
        assert_eq!(fix.source, DeviceLocationSource::Termux);
    }

    #[test]
    fn parse_fix_json_accuracy_optional_and_carries_source() {
        let fix = parse_fix_json(
            r#"{"latitude": 1.0, "longitude": 2.0}"#,
            DeviceLocationSource::CoreLocation,
        )
        .unwrap();
        assert_eq!(fix.accuracy_m, None);
        assert_eq!(fix.source, DeviceLocationSource::CoreLocation);
    }

    #[test]
    fn parse_fix_json_errors_without_coordinates() {
        // A location read that never got a fix comes back with nulls.
        assert!(
            parse_fix_json(
                r#"{"latitude": null, "longitude": null}"#,
                DeviceLocationSource::Termux
            )
            .is_err()
        );
        assert!(parse_fix_json("not json", DeviceLocationSource::Termux).is_err());
    }
}
