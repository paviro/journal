#![forbid(unsafe_code)]

//! Context providers: the external data sources sampled when an entry is written
//! — place (Nominatim geocoding + platform GPS), weather and air quality
//! (Open-Meteo), and celestial state (sun/moon). Each takes coordinates and a
//! time and returns a `notema-domain` value; none of this is local storage, so it
//! lives outside `notema-storage`.

use chrono::{DateTime, FixedOffset};
use notema_domain::{AirQuality, Celestial, Coordinates, Weather};

mod air;
mod celestial;
mod device_location;
mod error;
mod geocode;
mod http;
mod weather;

pub use air::fetch_air_quality;
pub use celestial::compute_celestial;
pub use device_location::{DeviceFix, DeviceLocationSource, device_location};
pub use error::{ContextError, Result};
pub use geocode::{GeocodeHit, geocode, reverse_geocode};
pub use weather::fetch_weather;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentProvider {
    Weather,
    AirQuality,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentWarning {
    pub provider: EnvironmentProvider,
    pub message: String,
}

/// The environment captured for one place and instant. Celestial data is local
/// and always present; independent network providers may return no observation
/// or record a warning without discarding the other results.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentReport {
    pub celestial: Celestial,
    pub weather: Option<Weather>,
    pub air_quality: Option<AirQuality>,
    pub warnings: Vec<EnvironmentWarning>,
}

pub fn fetch_environment(
    coordinates: Coordinates,
    datetime: DateTime<FixedOffset>,
) -> EnvironmentReport {
    let celestial = compute_celestial(coordinates, datetime);
    let mut warnings = Vec::new();
    let weather = fetch_weather(coordinates, datetime).unwrap_or_else(|error| {
        warnings.push(EnvironmentWarning {
            provider: EnvironmentProvider::Weather,
            message: error.to_string(),
        });
        None
    });
    let air_quality = fetch_air_quality(coordinates, datetime).unwrap_or_else(|error| {
        warnings.push(EnvironmentWarning {
            provider: EnvironmentProvider::AirQuality,
            message: error.to_string(),
        });
        None
    });

    EnvironmentReport {
        celestial,
        weather,
        air_quality,
        warnings,
    }
}
