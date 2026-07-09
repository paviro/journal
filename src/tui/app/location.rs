use super::*;
use crate::tui::geocode::{GeocodeQuery, GeocodeRequest};
use crate::tui::state::{EditLocationState, LocationPreset, LocationResolveStatus};
use chrono::{DateTime, FixedOffset};
use journal_storage::Location;
use std::collections::HashMap;

/// How many of the most-recent distinct places lead the preset list before it
/// fills out with the most-common ones.
const RECENT_PRESETS: usize = 5;
/// Upper bound on offered presets, so a large corpus doesn't flood the dialog.
const MAX_PRESETS: usize = 20;

/// Running aggregate for one distinct location while building presets.
struct PresetAgg {
    count: usize,
    last: Option<DateTime<FixedOffset>>,
    /// The location from the most-recent entry with this key, so the label and
    /// stored value reflect the latest naming.
    location: Location,
}

impl App {
    pub(crate) fn edit_location_state(&self) -> Option<&EditLocationState> {
        match &self.overlay {
            Overlay::EditLocation(state) => Some(state.as_ref()),
            _ => None,
        }
    }

    pub(crate) fn edit_location_state_mut(&mut self) -> Option<&mut EditLocationState> {
        match &mut self.overlay {
            Overlay::EditLocation(state) => Some(state.as_mut()),
            _ => None,
        }
    }

    pub(crate) fn begin_edit_location(&mut self) {
        let current = self
            .resolved_selected_entry()
            .and_then(|entry| entry.location.clone());
        let presets = self.location_presets();
        self.overlay = Overlay::EditLocation(Box::new(EditLocationState::new(current, presets)));
    }

    /// Existing locations offered as presets: the most-recent distinct places
    /// first, then the most-common ones not already shown. Keyed on the display
    /// label so case/format variants collapse. Archived journals don't contribute.
    pub(crate) fn location_presets(&self) -> Vec<LocationPreset> {
        let mut by_key: HashMap<String, PresetAgg> = HashMap::new();
        for entry in &self.library.entries {
            if journal_storage::is_archived_name(&entry.journal) {
                continue;
            }
            let Some(location) = &entry.location else {
                continue;
            };
            // A preset must be recognisable — skip locations that are nothing but
            // raw coordinates (a name, or a name-less address, is fine).
            if !location.has_named_parts() {
                continue;
            }
            let Some(label) = location.display_label() else {
                continue;
            };
            let when = entry.created_time();
            let agg = by_key
                .entry(label.to_lowercase())
                .or_insert_with(|| PresetAgg {
                    count: 0,
                    last: None,
                    location: location.clone(),
                });
            agg.count += 1;
            if when > agg.last {
                agg.last = when;
                agg.location = location.clone();
            }
        }

        let mut keys: Vec<String> = by_key.keys().cloned().collect();
        // Lead with the most-recent distinct places.
        keys.sort_by(|a, b| by_key[b].last.cmp(&by_key[a].last));
        let mut ordered: Vec<String> = keys.iter().take(RECENT_PRESETS).cloned().collect();
        // Fill the rest by usage frequency, skipping any recents already shown.
        let mut by_count = keys;
        by_count.sort_by(|a, b| {
            by_key[b]
                .count
                .cmp(&by_key[a].count)
                .then_with(|| by_key[b].last.cmp(&by_key[a].last))
        });
        for key in by_count {
            if ordered.len() >= MAX_PRESETS {
                break;
            }
            if !ordered.contains(&key) {
                ordered.push(key);
            }
        }

        ordered
            .into_iter()
            .map(|key| {
                let agg = &by_key[&key];
                LocationPreset {
                    label: agg.location.display_label().unwrap_or_default(),
                    location: agg.location.clone(),
                }
            })
            .collect()
    }

    /// Dispatch the dialog's current query to the geocode worker: coordinates are
    /// resolved immediately and enriched with names in the background; anything
    /// else is treated as an address. No-op on an empty query.
    pub(crate) fn resolve_location_query(&mut self) {
        let dispatch = {
            let Some(state) = self.edit_location_state_mut() else {
                return;
            };
            let query = state.query.trim().to_string();
            if query.is_empty() {
                return;
            }
            let id = state.next_request_id;
            state.next_request_id += 1;
            state.pending_request_id = Some(id);
            state.status = LocationResolveStatus::Resolving;
            let query = match parse_coordinates(&query) {
                Some((lat, lon)) => {
                    // Coordinates are already valid; keep them as the resolved value
                    // so the entry can be saved even before names come back.
                    let mut location = state.resolved.clone().unwrap_or_default();
                    location.latitude = Some(lat);
                    location.longitude = Some(lon);
                    state.resolved = Some(location);
                    GeocodeQuery::Coords { lat, lon }
                }
                None => GeocodeQuery::Address(query),
            };
            GeocodeRequest { id, query }
        };
        self.geocode.request(dispatch, crate::tui::geocode::resolve);
    }

    /// Ask the device for its current location, then name it like any coordinates.
    /// The grab and the reverse lookup both run on the worker thread, so the dialog
    /// just shows "Resolving…" until the fix (or a failure) comes back.
    pub(crate) fn grab_device_location(&mut self) {
        let dispatch = {
            let Some(state) = self.edit_location_state_mut() else {
                return;
            };
            let id = state.next_request_id;
            state.next_request_id += 1;
            state.pending_request_id = Some(id);
            state.status = LocationResolveStatus::Resolving;
            GeocodeRequest {
                id,
                query: GeocodeQuery::Device,
            }
        };
        self.geocode.request(dispatch, crate::tui::geocode::resolve);
    }

    /// Fold any finished geocode replies into the open dialog, ignoring stale ones
    /// (a reply whose id isn't the in-flight request). Returns whether anything
    /// changed, so the event loop knows to repaint.
    pub(crate) fn apply_geocode_results(&mut self) -> bool {
        let results = self.geocode.drain();
        let mut changed = false;
        for result in results {
            let Some(state) = self.edit_location_state_mut() else {
                continue;
            };
            if state.pending_request_id != Some(result.id) {
                continue;
            }
            state.pending_request_id = None;
            // A device grab returns its fix first: adopt the coordinates (and
            // accuracy/source) as the resolved, saveable value and mirror them
            // into the query field before the reverse-geocoded names land below.
            if let Some(fix) = &result.device_fix {
                state.seed_device_fix(fix);
            }
            match result.hits {
                Ok(hits) if result.reverse => state.apply_reverse(hits.into_iter().next()),
                Ok(hits) => state.apply_candidates(hits),
                Err(error) => state.status = LocationResolveStatus::Error(error),
            }
            changed = true;
        }
        changed
    }
}

/// Parse `"lat, lon"` into validated coordinates. `None` when it isn't two
/// comma-separated numbers within the valid latitude/longitude ranges — the
/// signal to treat the input as an address instead.
pub(crate) fn parse_coordinates(input: &str) -> Option<(f64, f64)> {
    let (lat, lon) = input.split_once(',')?;
    let lat: f64 = lat.trim().parse().ok()?;
    let lon: f64 = lon.trim().parse().ok()?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_coordinates_accepts_valid_and_rejects_out_of_range() {
        assert_eq!(parse_coordinates("52.52, 13.405"), Some((52.52, 13.405)));
        assert_eq!(parse_coordinates("  -33.8, 151.2 "), Some((-33.8, 151.2)));
        // Out of range.
        assert_eq!(parse_coordinates("91, 0"), None);
        assert_eq!(parse_coordinates("0, 181"), None);
        // Not two numbers.
        assert_eq!(parse_coordinates("Berlin"), None);
        assert_eq!(parse_coordinates("52.52"), None);
        assert_eq!(parse_coordinates("a, b"), None);
    }
}
