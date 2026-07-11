//! Context providers: the external data sources sampled when an entry is written
//! — place (Nominatim geocoding + platform GPS), weather and air quality
//! (Open-Meteo), and celestial state (sun/moon). Each takes coordinates and a
//! time and returns a `notema-core` value; none of this is local storage, so it
//! lives outside `notema-storage`.

mod air;
mod celestial;
mod device_location;
mod geocode;
mod http;
mod weather;

pub use notema_core::AppResult;

pub use air::fetch_air_quality;
pub use celestial::compute_celestial;
pub use device_location::{DeviceFix, DeviceLocationSource, device_location};
pub use geocode::{GeocodeHit, geocode, reverse_geocode};
pub use weather::fetch_weather;
