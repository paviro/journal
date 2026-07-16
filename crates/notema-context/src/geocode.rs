//! Address <-> coordinate geocoding via Nominatim (OpenStreetMap).
//!
//! Nominatim is free and keyless, but its usage policy requires a descriptive
//! `User-Agent`, at most one request per second, cached results, and forbids
//! per-keystroke autocomplete. This module caches every lookup for the process
//! lifetime; callers (the TUI location dialog) only ever geocode on an explicit
//! user action, never per keystroke.

use crate::Result;
use crate::http::get;
use notema_domain::{Coordinates, Location};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

const ENDPOINT_SEARCH: &str = "https://nominatim.openstreetmap.org/search";
const ENDPOINT_REVERSE: &str = "https://nominatim.openstreetmap.org/reverse";

/// One resolved place: coordinates plus a coarse-to-fine name hierarchy, and the
/// full human-readable name Nominatim returns for disambiguation in a list.
#[derive(Debug, Clone, PartialEq)]
pub struct GeocodeHit {
    pub display_name: String,
    pub location: Location,
    /// The IANA timezone of the matched OSM object, when it carries the tag
    /// (`extratags.timezone`). Often absent for a specific point — callers fall
    /// back to an offline coordinate lookup. Not persisted onto [`Location`].
    pub timezone: Option<String>,
}

/// Resolve a free-form address to candidate places (best match first). Empty
/// query yields no hits without a network call. Results are cached per query.
pub fn geocode(query: &str, limit: usize) -> Result<Vec<GeocodeHit>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let key = trimmed.to_lowercase();
    if let Some(hits) = cache().lock().unwrap().get(&key) {
        return Ok(hits.clone());
    }
    let url = format!(
        "{ENDPOINT_SEARCH}?format=jsonv2&addressdetails=1&extratags=1&limit={limit}&q={}",
        encode_component(trimmed)
    );
    let hits = parse_search_json(&get(&url)?)?;
    cache().lock().unwrap().insert(key, hits.clone());
    Ok(hits)
}

/// Reverse-resolve coordinates to a named place, filling locality/area/country.
/// `None` when Nominatim knows nothing there. Cached per rounded coordinate.
pub fn reverse_geocode(coordinates: Coordinates) -> Result<Option<GeocodeHit>> {
    let lat = coordinates.latitude();
    let lon = coordinates.longitude();
    let key = format!("@{lat:.5},{lon:.5}");
    if let Some(hits) = cache().lock().unwrap().get(&key) {
        return Ok(hits.first().cloned());
    }
    let url = format!(
        "{ENDPOINT_REVERSE}?format=jsonv2&addressdetails=1&extratags=1&lat={lat}&lon={lon}"
    );
    let hit = parse_reverse_json(&get(&url)?)?;
    cache()
        .lock()
        .unwrap()
        .insert(key, hit.clone().into_iter().collect());
    Ok(hit)
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
    /// `Option` so an explicit `"address": null` deserializes to `None` — Nominatim
    /// sends that for some results, and `#[serde(default)]` alone rejects a null.
    #[serde(default)]
    address: Option<NominatimAddress>,
    /// Free-form OSM tags of the matched object; we only read `timezone`, which
    /// many admin boundaries carry but most specific points don't. `Option` because
    /// Nominatim returns `"extratags": null` for places without any — and a bare
    /// `#[serde(default)]` would fail to parse that null, dropping the whole result.
    #[serde(default)]
    extratags: Option<NominatimExtratags>,
}

/// The `extratags` keys we read from a Nominatim place — just the IANA timezone.
#[derive(Deserialize, Default)]
struct NominatimExtratags {
    timezone: Option<String>,
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

/// Parse a forward-search body into hits. A body we can't deserialize is an error,
/// not an empty result — so a throttle/error page surfaces as a failure instead of a
/// misleading "no matches" (and isn't cached as empty by the caller).
fn parse_search_json(body: &str) -> Result<Vec<GeocodeHit>> {
    Ok(serde_json::from_str::<Vec<NominatimPlace>>(body)?
        .into_iter()
        .filter_map(place_to_hit)
        .collect())
}

/// Parse a reverse body into the zero-or-one hit. Nominatim answers an unknown point
/// with an `{"error": …}` object, which deserializes to a place with no fields and so
/// resolves to `None`; only a malformed body is an error.
fn parse_reverse_json(body: &str) -> Result<Option<GeocodeHit>> {
    Ok(place_to_hit(serde_json::from_str::<NominatimPlace>(body)?))
}

fn place_to_hit(place: NominatimPlace) -> Option<GeocodeHit> {
    let timezone = place
        .extratags
        .and_then(|extra| extra.timezone)
        .filter(|zone| !zone.is_empty());
    let address = place.address.unwrap_or_default();
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
        // A geocoded place has no device accuracy or provider.
        accuracy_m: None,
        source: None,
    };
    if location.is_empty() {
        return None;
    }
    Some(GeocodeHit {
        display_name: place.display_name.unwrap_or_default(),
        location,
        timezone,
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

        let hits = parse_search_json(body).unwrap();
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
        let hit = parse_reverse_json(body).unwrap().unwrap();
        assert_eq!(hit.location.city.as_deref(), Some("New York"));
        assert_eq!(hit.location.country.as_deref(), Some("United States"));
        // No extratags block at all -> no timezone.
        assert_eq!(hit.timezone, None);
    }

    #[test]
    fn parse_reverse_json_reads_extratags_timezone() {
        let with_tz = r#"{"lat":"35.68","lon":"139.76","display_name":"Tokyo",
            "address":{"city":"Tokyo","country":"Japan"},
            "extratags":{"timezone":"Asia/Tokyo","population":"14000000"}}"#;
        assert_eq!(
            parse_reverse_json(with_tz)
                .unwrap()
                .unwrap()
                .timezone
                .as_deref(),
            Some("Asia/Tokyo")
        );

        // An empty timezone tag is treated as absent.
        let empty_tz = r#"{"lat":"35.68","lon":"139.76","display_name":"Tokyo",
            "address":{"city":"Tokyo","country":"Japan"},"extratags":{"timezone":""}}"#;
        assert_eq!(
            parse_reverse_json(empty_tz).unwrap().unwrap().timezone,
            None
        );
    }

    #[test]
    fn parse_reverse_json_none_on_error_payload() {
        assert!(
            parse_reverse_json(r#"{"error":"Unable to geocode"}"#)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn parse_keeps_places_with_null_extratags_and_address() {
        // Nominatim returns `extratags: null` (and can return `address: null`) for many
        // specific places. A bare `#[serde(default)]` rejects a JSON null, so before the
        // fix the whole response failed to parse and every hit was silently dropped.
        let body = r#"[
            {"lat":"48.35","lon":"7.75","display_name":"22, Im Rheingarten, Schwanau",
             "address":{"house_number":"22","road":"Im Rheingarten","village":"Nonnenweier",
                        "postcode":"77963","country":"Deutschland"},
             "extratags":null},
            {"lat":"52.0","lon":"13.0","display_name":"Somewhere",
             "address":null,"extratags":null}
        ]"#;

        let hits = parse_search_json(body).unwrap();
        assert_eq!(
            hits.len(),
            2,
            "null extratags/address no longer drops the results"
        );
        assert_eq!(hits[0].location.road.as_deref(), Some("Im Rheingarten"));
        assert_eq!(hits[0].location.house_number.as_deref(), Some("22"));
        assert_eq!(hits[0].location.postcode.as_deref(), Some("77963"));
        assert_eq!(hits[0].timezone, None);
        // A place with a null address still keeps its coordinates.
        assert_eq!(hits[1].location.latitude, Some(52.0));
    }

    #[test]
    fn parse_search_json_surfaces_a_malformed_body_as_an_error() {
        // A non-array body (e.g. a throttle/error page) is a failure, not "no matches" —
        // so it can't be cached as empty or shown as a misleading empty result.
        assert!(parse_search_json("<html>rate limited</html>").is_err());
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
