//! Serde model for a Day One JSON export.
//!
//! The export is a `metadata` block plus an `entries` array. Media files
//! (photos/audios/videos/pdfs) live in sibling folders next to the JSON and are
//! named on disk by their `md5` (e.g. `photos/<md5>.<type>`), while the entry
//! body references them by `identifier` via `dayone-moment://<identifier>`.
//!
//! The model mirrors the export faithfully — location, weather, flags, per-entry
//! timezone, device/OS provenance, activity, and the full media tail — but the
//! importer still consumes only body text, tags, timestamps, and photos. The
//! rest is modeled deliberately so the import is *ready* to map it once the
//! journal format grows matching fields — hence the module-wide `dead_code`
//! allowance.
#![allow(dead_code)]

use serde::Deserialize;

/// Day One serializes some EXIF numbers inconsistently — a JSON number, a
/// stringified number, or the literal string `"(null)"`. Accept every shape and
/// yield `None` for anything not parseable as a float.
fn lenient_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

#[derive(Debug, Deserialize)]
pub struct DayOneExport {
    #[serde(default)]
    pub entries: Vec<DayOneEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayOneEntry {
    pub uuid: String,
    /// Markdown body. Day One escapes literal punctuation with backslashes.
    #[serde(default)]
    pub text: Option<String>,
    /// Clean structured body, present on newer entries. A JSON string (Day One's
    /// `ZRICHTEXTJSON`) that renders to faithful Markdown — preferred over `text`
    /// when available. See [`crate::dayone::richtext`].
    #[serde(default)]
    pub rich_text: Option<String>,
    /// RFC3339 (UTC) creation timestamp.
    pub creation_date: Option<String>,
    /// RFC3339 (UTC) last-modified timestamp.
    pub modified_date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,

    // --- Parsed but not imported yet (ready for future mapping) ---
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_all_day: bool,
    /// IANA name of the entry's authoring timezone, e.g. `Europe/Berlin`.
    pub time_zone: Option<String>,
    pub location: Option<Location>,
    pub weather: Option<Weather>,
    /// Auto-captured motion at write time (activity type plus step count).
    pub user_activity: Option<UserActivity>,

    // Provenance of the writing device/OS. Frequently null in older exports.
    pub creation_device: Option<String>,
    pub creation_device_model: Option<String>,
    pub creation_device_type: Option<String>,
    #[serde(rename = "creationOSName")]
    pub creation_os_name: Option<String>,
    #[serde(rename = "creationOSVersion")]
    pub creation_os_version: Option<String>,

    /// Total media duration in seconds (0 when there is no timed media).
    pub duration: Option<f64>,
    /// Seconds the author spent editing the entry.
    pub editing_time: Option<f64>,
    /// Rare, undocumented shape — kept as raw JSON so parsing stays lossless.
    pub template: Option<serde_json::Value>,

    // Media. Only `photos` are imported today; the rest are modeled with the
    // same fidelity so they can be ingested once the asset system supports
    // non-image files.
    #[serde(default)]
    pub photos: Vec<Moment>,
    #[serde(default)]
    pub audios: Vec<Moment>,
    #[serde(default)]
    pub videos: Vec<Moment>,
    #[serde(default)]
    pub pdf_attachments: Vec<Moment>,
}

/// Motion sample Day One attaches to an entry from device sensors.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserActivity {
    pub activity_name: Option<String>,
    pub step_count: Option<u64>,
}

/// A media attachment ("moment"). Referenced in the body by `identifier`, stored
/// on disk as `<folder>/<md5>.<type>` for typed media (photos). Audio has no
/// `type` and lives at `audios/<md5>.m4a` regardless of `format`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub identifier: String,
    pub md5: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub favorite: Option<bool>,
    pub date: Option<String>,
    /// Original file name and size on the source device.
    pub filename: Option<String>,
    pub file_size: Option<u64>,
    pub order_in_entry: Option<u32>,
    pub is_sketch: Option<bool>,
    pub creation_device: Option<String>,
    pub apple_cloud_identifier: Option<String>,
    /// Where the moment itself was captured (distinct from the entry's location).
    pub location: Option<Location>,
    pub time_zone_name: Option<String>,

    // Photo EXIF.
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub fnumber: Option<f64>,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub focal_length: Option<f64>,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub exposure_bias_value: Option<f64>,

    // Audio properties.
    pub format: Option<String>,
    pub audio_channels: Option<String>,
    pub sample_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub place_name: Option<String>,
    pub locality_name: Option<String>,
    pub administrative_area: Option<String>,
    pub country: Option<String>,
    pub time_zone_name: Option<String>,
    /// The geofence Day One recorded for the place (center plus radius).
    pub region: Option<Region>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub center: Option<Coordinate>,
    pub radius: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coordinate {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Weather {
    pub conditions_description: Option<String>,
    pub temperature_celsius: Option<f64>,
    pub weather_code: Option<String>,
    pub relative_humidity: Option<f64>,
    #[serde(rename = "windSpeedKPH")]
    pub wind_speed_kph: Option<f64>,
    pub weather_service_name: Option<String>,
    pub moon_phase: Option<f64>,
    pub moon_phase_code: Option<String>,
    #[serde(rename = "pressureMB")]
    pub pressure_mb: Option<f64>,
    #[serde(rename = "visibilityKM")]
    pub visibility_km: Option<f64>,
    pub sunrise_date: Option<String>,
    pub sunset_date: Option<String>,
    pub wind_bearing: Option<f64>,
    pub wind_chill_celsius: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One entry carrying the full field set, distilled from a real Day One
    /// export, to pin down that every modeled field deserializes.
    const FULL_ENTRY: &str = r#"{
      "entries": [
        {
          "uuid": "ABC123",
          "text": "Hello",
          "creationDate": "2021-04-03T06:30:05Z",
          "modifiedDate": "2021-04-03T06:30:05Z",
          "tags": ["dream"],
          "starred": true,
          "isPinned": false,
          "isAllDay": false,
          "timeZone": "Test/Zone",
          "duration": 0,
          "editingTime": 13.2,
          "creationDevice": "Test Device",
          "userActivity": { "activityName": "Biking", "stepCount": 10129 },
          "location": {
            "latitude": 10.0,
            "longitude": 20.0,
            "placeName": "1 Example Plaza",
            "localityName": "Testville",
            "administrativeArea": "Test Province",
            "country": "Testland",
            "region": { "center": { "latitude": 10.0, "longitude": 20.0 }, "radius": 75 }
          },
          "weather": {
            "conditionsDescription": "Rain",
            "temperatureCelsius": 10,
            "weatherCode": "rain-night",
            "weatherServiceName": "TestWeather",
            "pressureMB": 1013.2,
            "visibilityKM": 12.5,
            "moonPhase": 0.5
          },
          "audios": [
            {
              "identifier": "AUDIO0001",
              "md5": "00000000000000000000000000000001",
              "format": "aac",
              "audioChannels": "Mono",
              "sampleRate": "32.0 kHz",
              "duration": 106.336,
              "timeZoneName": "Test/Zone",
              "orderInEntry": 0,
              "fileSize": 670961
            }
          ],
          "photos": [
            {
              "identifier": "PHOTO0001",
              "md5": "00000000000000000000000000000002",
              "type": "jpeg",
              "width": 1600,
              "height": 1000,
              "filename": "IMG.jpg",
              "cameraModel": "Test Camera",
              "fnumber": "(null)",
              "focalLength": 4.2,
              "isSketch": false
            }
          ]
        }
      ]
    }"#;

    #[test]
    fn deserializes_full_field_set() {
        let export: DayOneExport = serde_json::from_str(FULL_ENTRY).unwrap();
        let entry = &export.entries[0];

        assert_eq!(entry.time_zone.as_deref(), Some("Test/Zone"));
        assert_eq!(entry.editing_time, Some(13.2));
        assert_eq!(
            entry.user_activity.as_ref().unwrap().step_count,
            Some(10129)
        );

        let region = entry.location.as_ref().unwrap().region.as_ref().unwrap();
        assert_eq!(region.radius, Some(75.0));
        assert_eq!(region.center.as_ref().unwrap().latitude, Some(10.0));

        let weather = entry.weather.as_ref().unwrap();
        assert_eq!(weather.pressure_mb, Some(1013.2));
        assert_eq!(weather.weather_service_name.as_deref(), Some("TestWeather"));

        let audio = &entry.audios[0];
        assert_eq!(audio.format.as_deref(), Some("aac"));
        assert_eq!(audio.sample_rate.as_deref(), Some("32.0 kHz"));
        assert_eq!(audio.kind, None);

        let photo = &entry.photos[0];
        // `fnumber` arrives as the string "(null)" — lenient parse yields None,
        // while a real numeric `focalLength` still parses.
        assert_eq!(photo.fnumber, None);
        assert_eq!(photo.focal_length, Some(4.2));
        assert_eq!(photo.camera_model.as_deref(), Some("Test Camera"));
    }
}
