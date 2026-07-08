//! Address <-> coordinate geocoding via Nominatim (OpenStreetMap).
//!
//! Nominatim is free and keyless, but its usage policy requires a descriptive
//! `User-Agent`, at most one request per second, cached results, and forbids
//! per-keystroke autocomplete. This module caches every lookup for the process
//! lifetime; callers (the TUI location dialog) only ever geocode on an explicit
//! user action, never per keystroke.

use crate::AppResult;
use journal_core::Location;
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

const ENDPOINT_SEARCH: &str = "https://nominatim.openstreetmap.org/search";
const ENDPOINT_REVERSE: &str = "https://nominatim.openstreetmap.org/reverse";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound on a response body (bytes) — geocoding JSON is tiny.
const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;
/// Identifies the application to Nominatim, as its policy requires (a stock HTTP
/// library User-Agent is explicitly rejected).
const USER_AGENT: &str = concat!("journal-tui/", env!("CARGO_PKG_VERSION"));

/// One resolved place: coordinates plus a coarse-to-fine name hierarchy, and the
/// full human-readable name Nominatim returns for disambiguation in a list.
#[derive(Debug, Clone, PartialEq)]
pub struct GeocodeHit {
    pub display_name: String,
    pub location: Location,
}

/// Resolve a free-form address to candidate places (best match first). Empty
/// query yields no hits without a network call. Results are cached per query.
pub fn geocode(query: &str, limit: usize) -> AppResult<Vec<GeocodeHit>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let key = trimmed.to_lowercase();
    if let Some(hits) = cache().lock().unwrap().get(&key) {
        return Ok(hits.clone());
    }
    let url = format!(
        "{ENDPOINT_SEARCH}?format=jsonv2&addressdetails=1&limit={limit}&q={}",
        encode_component(trimmed)
    );
    let hits = parse_search_json(&get(&url)?);
    cache().lock().unwrap().insert(key, hits.clone());
    Ok(hits)
}

/// Reverse-resolve coordinates to a named place, filling locality/area/country.
/// `None` when Nominatim knows nothing there. Cached per rounded coordinate.
pub fn reverse_geocode(lat: f64, lon: f64) -> AppResult<Option<GeocodeHit>> {
    let key = format!("@{lat:.5},{lon:.5}");
    if let Some(hits) = cache().lock().unwrap().get(&key) {
        return Ok(hits.first().cloned());
    }
    let url = format!("{ENDPOINT_REVERSE}?format=jsonv2&addressdetails=1&lat={lat}&lon={lon}");
    let hit = parse_reverse_json(&get(&url)?);
    cache()
        .lock()
        .unwrap()
        .insert(key, hit.clone().into_iter().collect());
    Ok(hit)
}

fn get(url: &str) -> AppResult<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let body = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()?
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()?;
    Ok(body)
}

/// Per-process geocode cache. Forward lookups key on the lowercased query,
/// reverse lookups on a rounded `@lat,lon`; both store 0+ hits so a repeated
/// resolve never re-hits the network (satisfying Nominatim's caching rule).
fn cache() -> &'static Mutex<HashMap<String, Vec<GeocodeHit>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<GeocodeHit>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Percent-encode a query-string component (RFC 3986 unreserved set kept as-is).
fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The subset of a Nominatim jsonv2 place we map onto [`Location`].
#[derive(Deserialize)]
struct NominatimPlace {
    lat: Option<String>,
    lon: Option<String>,
    display_name: Option<String>,
    /// The primary object's own name (a shop, venue, landmark, …), when it has
    /// one. Empty/absent for a plain address.
    name: Option<String>,
    #[serde(default)]
    address: NominatimAddress,
}

/// The Nominatim `address` keys we mirror onto [`Location`], one-to-one.
#[derive(Deserialize, Default)]
struct NominatimAddress {
    house_number: Option<String>,
    road: Option<String>,
    neighbourhood: Option<String>,
    quarter: Option<String>,
    suburb: Option<String>,
    borough: Option<String>,
    city_district: Option<String>,
    city: Option<String>,
    town: Option<String>,
    village: Option<String>,
    municipality: Option<String>,
    hamlet: Option<String>,
    postcode: Option<String>,
    county: Option<String>,
    state_district: Option<String>,
    province: Option<String>,
    region: Option<String>,
    state: Option<String>,
    country: Option<String>,
}

fn parse_search_json(body: &str) -> Vec<GeocodeHit> {
    serde_json::from_str::<Vec<NominatimPlace>>(body)
        .unwrap_or_default()
        .into_iter()
        .filter_map(place_to_hit)
        .collect()
}

fn parse_reverse_json(body: &str) -> Option<GeocodeHit> {
    serde_json::from_str::<NominatimPlace>(body)
        .ok()
        .and_then(place_to_hit)
}

fn place_to_hit(place: NominatimPlace) -> Option<GeocodeHit> {
    let address = place.address;
    let location = Location {
        // A named POI (shop, venue, landmark) carries its name; a plain address
        // doesn't, leaving the user to supply one.
        name: place.name.filter(|name| !name.is_empty()),
        house_number: address.house_number,
        road: address.road,
        neighbourhood: address.neighbourhood,
        quarter: address.quarter,
        suburb: address.suburb,
        borough: address.borough,
        city_district: address.city_district,
        city: address.city,
        town: address.town,
        village: address.village,
        municipality: address.municipality,
        hamlet: address.hamlet,
        postcode: address.postcode,
        county: address.county,
        state_district: address.state_district,
        province: address.province,
        region: address.region,
        state: address.state,
        country: address.country,
        latitude: place.lat.as_deref().and_then(|value| value.parse().ok()),
        longitude: place.lon.as_deref().and_then(|value| value.parse().ok()),
    };
    if location.is_empty() {
        return None;
    }
    Some(GeocodeHit {
        display_name: place.display_name.unwrap_or_default(),
        location,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_json_maps_fields_and_drops_empty() {
        let body = r#"[
            {"lat":"52.5200","lon":"13.4050","name":"Corner Cafe",
             "display_name":"Corner Cafe, Bahnhofstraße 1, Berlin",
             "address":{"road":"Bahnhofstraße","house_number":"1","suburb":"Mitte",
                        "postcode":"10115","city":"Berlin","state":"Berlin",
                        "country":"Deutschland"}},
            {"lat":"48.1","lon":"11.6","name":"","display_name":"Munich",
             "address":{"town":"Munich","county":"Upper Bavaria","country":"Germany"}},
            {"display_name":"Nowhere","address":{}}
        ]"#;

        let hits = parse_search_json(body);
        assert_eq!(hits.len(), 2, "the address-less place is dropped");
        // Each OSM key lands in its own field, verbatim.
        assert_eq!(hits[0].location.name.as_deref(), Some("Corner Cafe"));
        assert_eq!(hits[0].location.road.as_deref(), Some("Bahnhofstraße"));
        assert_eq!(hits[0].location.house_number.as_deref(), Some("1"));
        assert_eq!(hits[0].location.suburb.as_deref(), Some("Mitte"));
        assert_eq!(hits[0].location.postcode.as_deref(), Some("10115"));
        assert_eq!(hits[0].location.city.as_deref(), Some("Berlin"));
        assert_eq!(hits[0].location.state.as_deref(), Some("Berlin"));
        assert_eq!(hits[0].location.country.as_deref(), Some("Deutschland"));
        assert_eq!(hits[0].location.latitude, Some(52.52));
        assert_eq!(hits[0].location.longitude, Some(13.405));
        // An empty `name` is dropped; `town` and `county` keep their own fields.
        assert_eq!(hits[1].location.name, None);
        assert_eq!(hits[1].location.town.as_deref(), Some("Munich"));
        assert_eq!(hits[1].location.city, None);
        assert_eq!(hits[1].location.county.as_deref(), Some("Upper Bavaria"));
    }

    #[test]
    fn parse_reverse_json_reads_single_object() {
        let body = r#"{"lat":"40.71","lon":"-74.01","display_name":"New York",
            "address":{"city":"New York","state":"New York","country":"United States"}}"#;
        let hit = parse_reverse_json(body).unwrap();
        assert_eq!(hit.location.city.as_deref(), Some("New York"));
        assert_eq!(hit.location.country.as_deref(), Some("United States"));
    }

    #[test]
    fn parse_reverse_json_none_on_error_payload() {
        assert!(parse_reverse_json(r#"{"error":"Unable to geocode"}"#).is_none());
    }

    #[test]
    fn encode_component_escapes_spaces_and_punctuation() {
        assert_eq!(
            encode_component("1 Main St, Berlin"),
            "1%20Main%20St%2C%20Berlin"
        );
        assert_eq!(encode_component("caf\u{e9}"), "caf%C3%A9");
    }
}
