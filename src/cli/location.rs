//! Resolve the `notema log --location` flag into a place, doing all blocking GPS
//! and network work here — before any fullscreen editor takes over the screen —
//! so a device grab, geocode, or disambiguation picker prints cleanly.
//!
//! Three modes mirror the TUI location dialog's single field: a bare flag grabs a
//! device fix, a `lat,lon` value is reverse-geocoded, anything else is treated as
//! an address to forward-geocode. When several places match an address, an
//! interactive session picks from a numbered list; a piped one takes the top hit.

use std::io::{self, Write};

use anyhow::bail;
use notema_context::{DeviceFix, GeocodeHit, device_location, geocode, reverse_geocode};
use notema_domain::{Coordinates, Location};

use crate::AppResult;

/// How many candidates to request for an ambiguous address before ranking.
const GEOCODE_LIMIT: usize = 8;

/// A resolved place plus the IANA timezone the geocoder attached to it, if any
/// (fed to `resolve_zone` so a located entry adopts its place's zone).
pub(super) struct ResolvedLocation {
    pub location: Location,
    pub osm_timezone: Option<String>,
}

/// Turn the `--location` flag into a place. `None` (flag absent) yields `Ok(None)`;
/// `Some(None)` (bare flag) grabs a device fix; `Some(Some(value))` is a `lat,lon`
/// pair or an address. `interactive` is true when a numbered picker can be shown
/// and answered — stdout is a terminal and stdin isn't the piped entry body.
pub(super) fn resolve(
    spec: Option<Option<String>>,
    interactive: bool,
) -> AppResult<Option<ResolvedLocation>> {
    let resolved = match spec {
        None => return Ok(None),
        Some(None) => resolve_device()?,
        Some(Some(value)) => match Coordinates::parse(&value) {
            Some(coordinates) => {
                let base = Location {
                    latitude: Some(coordinates.latitude()),
                    longitude: Some(coordinates.longitude()),
                    ..Location::default()
                };
                fold_reverse(base)
            }
            None => resolve_address(&value, interactive)?,
        },
    };
    if let Some(label) = resolved.location.display_label() {
        eprintln!("location: {label}");
    }
    Ok(Some(resolved))
}

/// Grab a device GPS fix and name it. The raw coordinates stay saveable even when
/// the reverse lookup finds nothing (offline, or an unmapped spot).
fn resolve_device() -> AppResult<ResolvedLocation> {
    let fix = device_location()?;
    let base = base_from_fix(&fix);
    Ok(fold_reverse(base))
}

/// The coordinates-only location a device fix resolves to before naming: its
/// lat/lon plus the fix's accuracy and provider slug.
fn base_from_fix(fix: &DeviceFix) -> Location {
    let mut location = Location {
        accuracy_m: fix.accuracy_m,
        source: Some(fix.source.to_string()),
        ..Location::default()
    };
    location.set_coordinates(fix.coordinates);
    location
}

/// Reverse-geocode a coordinates-only `base` and overlay the returned names,
/// keeping `base`'s exact coordinates, accuracy, and provider. A failed or empty
/// lookup leaves the bare coordinates — still a saveable location.
fn fold_reverse(base: Location) -> ResolvedLocation {
    let Some(coordinates) = base.coordinates() else {
        return ResolvedLocation {
            location: base,
            osm_timezone: None,
        };
    };
    match reverse_geocode(coordinates) {
        Ok(Some(hit)) => ResolvedLocation {
            location: hit.location.with_pin_from(&base),
            osm_timezone: hit.timezone,
        },
        Ok(None) => {
            eprintln!("note: no place name found for these coordinates; saving them as-is");
            ResolvedLocation {
                location: base,
                osm_timezone: None,
            }
        }
        Err(error) => {
            eprintln!("note: reverse geocoding failed ({error}); saving the coordinates as-is");
            ResolvedLocation {
                location: base,
                osm_timezone: None,
            }
        }
    }
}

/// Forward-geocode an address and pick one candidate: the sole hit, an
/// interactively chosen one, or (piped) the top-ranked hit.
fn resolve_address(query: &str, interactive: bool) -> AppResult<ResolvedLocation> {
    let mut hits = geocode(query, GEOCODE_LIMIT)?;
    if hits.is_empty() {
        bail!("no place found for \"{query}\"");
    }
    let hit = if hits.len() == 1 || !interactive {
        hits.swap_remove(0)
    } else {
        let index = pick_hit(&hits)?;
        hits.swap_remove(index)
    };
    Ok(ResolvedLocation {
        location: hit.location,
        osm_timezone: hit.timezone,
    })
}

/// Prompt for a choice among ambiguous candidates, re-asking on bad input. The
/// list and prompt go to stderr so stdout stays the created entry's path; an empty
/// answer (bare Enter) takes the top hit.
fn pick_hit(hits: &[GeocodeHit]) -> AppResult<usize> {
    let stderr = io::stderr();
    let mut stderr = stderr.lock();
    for (index, hit) in hits.iter().enumerate() {
        writeln!(stderr, "  {}) {}", index + 1, hit.display_name)?;
    }
    loop {
        write!(stderr, "Select [1]: ")?;
        stderr.flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            // EOF (stdin closed) — fall back to the top hit rather than loop.
            return Ok(0);
        }
        if let Some(index) = parse_selection(&input, hits.len()) {
            return Ok(index);
        }
        writeln!(stderr, "Enter a number between 1 and {}.", hits.len())?;
    }
}

/// Parse a picker answer into a 0-based index. An empty line is the default (top
/// hit); a number in `1..=count` selects it; anything else is `None` (re-prompt).
fn parse_selection(input: &str, count: usize) -> Option<usize> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Some(0);
    }
    let choice: usize = trimmed.parse().ok()?;
    (1..=count).contains(&choice).then(|| choice - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_selection_defaults_empty_to_top_hit() {
        assert_eq!(parse_selection("", 3), Some(0));
        assert_eq!(parse_selection("  \n", 3), Some(0));
    }

    #[test]
    fn parse_selection_maps_valid_numbers_one_based() {
        assert_eq!(parse_selection("1", 3), Some(0));
        assert_eq!(parse_selection("3\n", 3), Some(2));
    }

    #[test]
    fn parse_selection_rejects_out_of_range_and_garbage() {
        assert_eq!(parse_selection("0", 3), None);
        assert_eq!(parse_selection("4", 3), None);
        assert_eq!(parse_selection("two", 3), None);
        assert_eq!(parse_selection("-1", 3), None);
    }
}
