//! Air quality (and UV) for an entry's place and date via Open-Meteo's
//! air-quality API — free and keyless, a sibling to the weather fetch.
//!
//! A single endpoint serves both the recent past and a short forecast; there is
//! no archive/forecast split like the weather API. Its reanalysis reaches back
//! only a few years and its past window is bounded (about three months), so
//! entries older than that — most Day One imports — simply get no reading and
//! the `[air_quality]` table is omitted.

use crate::Result;
use crate::http::get;
use crate::weather::nearest_hour_index;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use notema_domain::{AirQuality, Coordinates};
use serde::Deserialize;

const ENDPOINT: &str = "https://air-quality-api.open-meteo.com/v1/air-quality";
/// The hourly variables we map onto [`AirQuality`]. `timezone=auto` makes the
/// returned `time[]` local to the coordinates, matching the entry's wall clock.
const HOURLY: &str = "pm2_5,pm10,ozone,nitrogen_dioxide,sulphur_dioxide,carbon_monoxide,european_aqi,us_aqi,uv_index,grass_pollen,birch_pollen,ragweed_pollen";
/// Attribution stored in the `[air_quality]` table's `source`.
const SOURCE: &str = "Open-Meteo";

/// Fetch the air quality for a point at an instant. `Ok(None)` when Open-Meteo
/// has no usable sample there/then (date outside the endpoint's window, or a gap
/// in the data). Errors are transport/HTTP failures — the caller drops them
/// silently so a save never fails.
pub fn fetch_air_quality(
    coordinates: Coordinates,
    datetime: DateTime<FixedOffset>,
) -> Result<Option<AirQuality>> {
    let lat = coordinates.latitude();
    let lon = coordinates.longitude();
    let date = datetime.format("%Y-%m-%d");
    let url = format!(
        "{ENDPOINT}?latitude={lat}&longitude={lon}&timezone=auto\
         &start_date={date}&end_date={date}&hourly={HOURLY}"
    );
    let response: OpenMeteoResponse = serde_json::from_str(&get(&url)?)?;
    Ok(response
        .hourly
        .and_then(|hourly| extract_air_quality(&hourly, datetime.naive_local())))
}

/// The hourly block of an air-quality response. Every series is optional and
/// holds optional values — a variable can be missing for a region (pollen is
/// Europe-only) and any hour can be a gap.
#[derive(Deserialize)]
struct OpenMeteoResponse {
    hourly: Option<Hourly>,
}

#[derive(Deserialize)]
struct Hourly {
    /// Local timestamps, one per hour, e.g. `"2026-07-08T14:00"`.
    time: Vec<String>,
    pm2_5: Option<Vec<Option<f64>>>,
    pm10: Option<Vec<Option<f64>>>,
    ozone: Option<Vec<Option<f64>>>,
    nitrogen_dioxide: Option<Vec<Option<f64>>>,
    sulphur_dioxide: Option<Vec<Option<f64>>>,
    carbon_monoxide: Option<Vec<Option<f64>>>,
    european_aqi: Option<Vec<Option<f64>>>,
    us_aqi: Option<Vec<Option<f64>>>,
    uv_index: Option<Vec<Option<f64>>>,
    grass_pollen: Option<Vec<Option<f64>>>,
    birch_pollen: Option<Vec<Option<f64>>>,
    ragweed_pollen: Option<Vec<Option<f64>>>,
}

/// Pull the sample nearest `target` from the hourly series and map it onto
/// [`AirQuality`]. `None` when there is no parseable hour or the nearest one
/// carries no data at all (so we don't persist a table holding only attribution).
fn extract_air_quality(hourly: &Hourly, target: NaiveDateTime) -> Option<AirQuality> {
    let index = nearest_hour_index(&hourly.time, target)?;
    let at = |series: &Option<Vec<Option<f64>>>| {
        series
            .as_ref()
            .and_then(|values| values.get(index).copied().flatten())
    };
    // The AQIs are integers; Open-Meteo returns them as floats, so round.
    let aqi = |series: &Option<Vec<Option<f64>>>| at(series).map(|value| value.round() as i64);

    let air = AirQuality {
        european_aqi: aqi(&hourly.european_aqi),
        us_aqi: aqi(&hourly.us_aqi),
        pm2_5: at(&hourly.pm2_5),
        pm10: at(&hourly.pm10),
        ozone: at(&hourly.ozone),
        nitrogen_dioxide: at(&hourly.nitrogen_dioxide),
        sulphur_dioxide: at(&hourly.sulphur_dioxide),
        carbon_monoxide: at(&hourly.carbon_monoxide),
        uv_index: at(&hourly.uv_index),
        grass_pollen: at(&hourly.grass_pollen),
        birch_pollen: at(&hourly.birch_pollen),
        ragweed_pollen: at(&hourly.ragweed_pollen),
        source: None,
    };

    if air.is_empty() {
        return None;
    }
    Some(AirQuality {
        source: Some(SOURCE.to_string()),
        ..air
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, crate::weather::OPEN_METEO_HOUR_FORMAT).unwrap()
    }

    // A minimal two-hour response covering 13:00 and 14:00 local. Pollen is
    // omitted, as Open-Meteo does outside Europe.
    const SAMPLE: &str = r#"{
        "hourly": {
            "time": ["2026-07-08T13:00", "2026-07-08T14:00"],
            "pm2_5": [8.0, 12.4],
            "pm10": [15.0, 20.0],
            "ozone": [60.0, 72.0],
            "nitrogen_dioxide": [10.0, 14.0],
            "sulphur_dioxide": [2.0, 3.0],
            "carbon_monoxide": [100.0, 120.0],
            "european_aqi": [30.4, 41.6],
            "us_aqi": [48.0, 55.0],
            "uv_index": [4.0, 6.2]
        }
    }"#;

    #[test]
    fn maps_nearest_hour_and_rounds_aqi() {
        let response: OpenMeteoResponse = serde_json::from_str(SAMPLE).unwrap();
        // 14:20 is nearest the 14:00 sample.
        let air =
            extract_air_quality(&response.hourly.unwrap(), naive("2026-07-08T14:20")).unwrap();

        assert_eq!(air.pm2_5, Some(12.4));
        assert_eq!(air.pm10, Some(20.0));
        assert_eq!(air.ozone, Some(72.0));
        assert_eq!(air.uv_index, Some(6.2));
        // Float AQIs round to the nearest integer.
        assert_eq!(air.european_aqi, Some(42));
        assert_eq!(air.us_aqi, Some(55));
        // Pollen absent outside Europe.
        assert_eq!(air.grass_pollen, None);
        assert_eq!(air.source.as_deref(), Some("Open-Meteo"));
    }

    #[test]
    fn missing_series_and_gaps_drop_empty() {
        // Some series omitted, others present but null at the hour.
        let body = r#"{"hourly":{"time":["2026-07-08T14:00"],"pm2_5":[null]}}"#;
        let response: OpenMeteoResponse = serde_json::from_str(body).unwrap();
        assert!(
            extract_air_quality(&response.hourly.unwrap(), naive("2026-07-08T14:00")).is_none()
        );
    }

    #[test]
    fn no_hours_yields_none() {
        let body = r#"{"hourly":{"time":[]}}"#;
        let response: OpenMeteoResponse = serde_json::from_str(body).unwrap();
        assert!(
            extract_air_quality(&response.hourly.unwrap(), naive("2026-07-08T14:00")).is_none()
        );
    }
}
