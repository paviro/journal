//! Coordinate -> timezone resolution for a located entry, ranked: an IANA name
//! the geocoder supplied first, then an offline lookup from the coordinates, then
//! nothing (the caller keeps the system zone). Stamping an entry with the zone of
//! where it was written — rather than the machine's — keeps travel from skewing
//! its local time, its date-folder, and its sunrise/sunset.

use std::sync::LazyLock;

use chrono::{DateTime, FixedOffset};
use chrono_tz::Tz;
use notema_domain::Coordinates;

// The offline coordinate->zone finder. Its dataset is embedded in the binary; the
// `exact-timezone` feature swaps the small tile-based finder for exact polygons
// (see Cargo.toml). Only the type named here is referenced, so the other finder's
// embedded data is dropped by dead-code elimination and never bloats the binary.
#[cfg(feature = "exact-timezone")]
type TzFinder = tzf_rs::DefaultFinder;
#[cfg(not(feature = "exact-timezone"))]
type TzFinder = tzf_rs::FuzzyFinder;

// Building a finder parses the embedded dataset, so keep one for the process.
static FINDER: LazyLock<TzFinder> = LazyLock::new(TzFinder::new);

/// The IANA zone the offline finder places `coordinates` in, if any. tzf-rs takes
/// longitude before latitude, and returns `""` for a point it can't place (open
/// ocean without maritime data); an empty or unparseable name yields `None`.
fn finder_zone(coordinates: Coordinates) -> Option<Tz> {
    FINDER
        .get_tz_name(coordinates.longitude(), coordinates.latitude())
        .parse()
        .ok()
}

/// Resolve the timezone for a located entry, preferring `osm_zone` (an IANA name
/// from the geocoder) over the offline lookup. `None` means unresolved — the
/// caller should keep the system zone.
pub fn resolve_zone(coordinates: Coordinates, osm_zone: Option<&str>) -> Option<Tz> {
    osm_zone
        .and_then(|name| name.parse().ok())
        .or_else(|| finder_zone(coordinates))
}

/// Re-express `datetime` in `zone`: the instant is unchanged, but the offset (and
/// so the wall-clock reading and the date) become those of `zone`.
pub fn rezone(datetime: DateTime<FixedOffset>, zone: Tz) -> DateTime<FixedOffset> {
    datetime.with_timezone(&zone).fixed_offset()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coords(latitude: f64, longitude: f64) -> Coordinates {
        Coordinates::try_new(latitude, longitude).unwrap()
    }

    #[test]
    fn finds_the_zone_for_a_land_point() {
        // Central Tokyo, well inside Japan — stable across both finders.
        assert_eq!(finder_zone(coords(35.68, 139.767)), Some(Tz::Asia__Tokyo));
    }

    #[test]
    fn passes_longitude_and_latitude_in_the_right_order() {
        // (139.767, 35.68) is Tokyo; the swapped pair (35.68, 139.767) is an
        // invalid longitude, so a mixed-up call could never yield Asia/Tokyo.
        assert_eq!(finder_zone(coords(35.68, 139.767)), Some(Tz::Asia__Tokyo));
    }

    #[test]
    fn prefers_the_osm_zone_over_the_coordinates() {
        // Tokyo coordinates but an OSM-supplied Berlin zone — OSM wins.
        assert_eq!(
            resolve_zone(coords(35.68, 139.767), Some("Europe/Berlin")),
            Some(Tz::Europe__Berlin)
        );
    }

    #[test]
    fn falls_back_to_the_finder_when_osm_is_absent_or_unparseable() {
        let tokyo = coords(35.68, 139.767);
        assert_eq!(resolve_zone(tokyo, None), Some(Tz::Asia__Tokyo));
        assert_eq!(
            resolve_zone(tokyo, Some("Not/AZone")),
            Some(Tz::Asia__Tokyo)
        );
        assert_eq!(resolve_zone(tokyo, Some("")), Some(Tz::Asia__Tokyo));
    }

    #[test]
    fn rezone_keeps_the_instant_but_takes_the_zone_offset() {
        let utc = DateTime::parse_from_rfc3339("2026-07-16T00:30:00+00:00").unwrap();
        let tokyo = rezone(utc, Tz::Asia__Tokyo);
        // Same instant, Tokyo's summer offset (+09:00), so the wall clock and date roll forward.
        assert_eq!(tokyo, utc);
        assert_eq!(tokyo.offset().local_minus_utc(), 9 * 3600);
        assert_eq!(
            tokyo.format("%Y-%m-%d %H:%M").to_string(),
            "2026-07-16 09:30"
        );
    }
}
