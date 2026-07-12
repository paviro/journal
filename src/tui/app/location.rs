use super::*;
use crate::tui::geocode::{GeocodeQuery, GeocodeRequest};
use crate::tui::state::{ListNav, SelectableList};
use chrono::{DateTime, FixedOffset};
use notema_context::{DeviceFix, GeocodeHit};
use notema_domain::Location;
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
        if self.editor.is_none() && !self.allow_selected_entry_edit() {
            return;
        }
        let current = self.editing_location();
        let presets = self.location_presets();
        self.overlay = Overlay::EditLocation(Box::new(EditLocationState::new(current, presets)));
    }

    /// Existing locations offered as presets: the most-recent distinct places
    /// first, then the most-common ones not already shown. Keyed on the display
    /// label so case/format variants collapse. Archived journals don't contribute.
    pub(crate) fn location_presets(&self) -> Vec<LocationPreset> {
        let mut by_key: HashMap<String, PresetAgg> = HashMap::new();
        for entry in &self.library.entries {
            if notema_storage::is_archived_name(&entry.journal) {
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
        let id = self.next_geocode_id;
        let dispatch = {
            let Some(state) = self.edit_location_state_mut() else {
                return;
            };
            let query = state.query.as_str().trim().to_string();
            if query.is_empty() {
                return;
            }
            state.pending_request_id = Some(id);
            state.status = LocationResolveStatus::Resolving;
            let query = match parse_coordinates(&query) {
                Some(coordinates) => {
                    // Coordinates are already valid; keep them as the resolved value
                    // so the entry can be saved even before names come back.
                    let mut location = state.resolved.clone().unwrap_or_default();
                    location.set_coordinates(coordinates);
                    state.resolved = Some(location);
                    GeocodeQuery::Coordinates(coordinates)
                }
                None => GeocodeQuery::Address(query),
            };
            GeocodeRequest { id, query }
        };
        self.next_geocode_id += 1;
        self.geocode.request(dispatch, crate::tui::geocode::resolve);
    }

    /// Ask the device for its current location, then name it like any coordinates.
    /// The grab and the reverse lookup both run on the worker thread, so the dialog
    /// just shows "Resolving…" until the fix (or a failure) comes back.
    pub(crate) fn grab_device_location(&mut self) {
        let id = self.next_geocode_id;
        let dispatch = {
            let Some(state) = self.edit_location_state_mut() else {
                return;
            };
            state.pending_request_id = Some(id);
            state.status = LocationResolveStatus::Resolving;
            GeocodeRequest {
                id,
                query: GeocodeQuery::Device,
            }
        };
        self.next_geocode_id += 1;
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

/// Which field of the location dialog has keyboard focus.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditLocationFocus {
    #[default]
    Query,
    Name,
    List,
}

/// Progress of an on-demand geocode lookup, surfaced as the dialog's status line.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) enum LocationResolveStatus {
    #[default]
    Idle,
    Resolving,
    Resolved,
    NoMatch,
    Error(String),
}

/// A recent/most-common existing location offered as a preset. `label` is its
/// display line; `location` is copied wholesale when the preset is chosen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocationPreset {
    pub(crate) label: String,
    pub(crate) location: Location,
}

/// State for the location overlay: two text fields (a free-form address or
/// coordinate query, and a place label) plus a list that shows geocode candidate
/// matches once a lookup returns, or recent/common presets otherwise. Geocoding
/// is dispatched to a background worker; `pending_request_id` guards against a
/// stale reply landing after a newer request.
pub(crate) struct EditLocationState {
    /// Free-form address, or `"lat, lon"` coordinates.
    pub(crate) query: TextInput,
    /// The place's human label — maps to [`Location::name`].
    pub(crate) name: TextInput,
    /// Coordinates + names resolved from the query, a candidate, or a preset.
    pub(crate) resolved: Option<Location>,
    /// Recent-then-common existing locations, shown when no lookup is active.
    pub(crate) presets: Vec<LocationPreset>,
    /// Candidate matches from the last forward geocode; while non-empty they
    /// replace the presets in the list.
    pub(crate) candidates: Vec<GeocodeHit>,
    pub(crate) list: SelectableList,
    pub(crate) focus: EditLocationFocus,
    pub(crate) status: LocationResolveStatus,
    /// Whether the current query text has already been looked up. When set, Enter
    /// in the address field commits instead of re-querying; editing the query
    /// clears it (and the shown result), flipping Enter back to "look up".
    pub(crate) query_looked_up: bool,
    /// The in-flight request's id (allocated from the app-level counter) so a late
    /// reply for an older query can be dropped.
    pub(crate) pending_request_id: Option<u64>,
}

impl EditLocationState {
    pub(crate) fn new(current: Option<Location>, presets: Vec<LocationPreset>) -> Self {
        let name = current
            .as_ref()
            .and_then(|loc| loc.name.clone())
            .unwrap_or_default();
        let query = current.as_ref().map(query_seed).unwrap_or_default();
        // Treat the seeded query as already looked up only when the stored location
        // carries address detail — a bare coordinate pair still needs a lookup, so
        // Enter should resolve it rather than save.
        let query_looked_up = !query.is_empty()
            && current
                .as_ref()
                .is_some_and(|location| location.has_named_parts());
        let mut state = Self {
            query_looked_up,
            query: TextInput::from(query),
            name: TextInput::from(name),
            resolved: current,
            presets,
            candidates: Vec::new(),
            list: SelectableList::default(),
            focus: EditLocationFocus::Query,
            status: LocationResolveStatus::Idle,
            pending_request_id: None,
        };
        state.normalize_list_state();
        state
    }

    /// The list shows geocode candidates once a lookup returns them, else presets.
    pub(crate) fn showing_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    /// The labels currently shown in the list — candidate matches (our parsed
    /// label, falling back to the raw display name) or preset labels.
    pub(crate) fn list_labels(&self) -> Vec<String> {
        if self.showing_candidates() {
            self.candidates
                .iter()
                .map(|hit| {
                    hit.location
                        .display_label()
                        .unwrap_or_else(|| hit.display_name.clone())
                })
                .collect()
        } else {
            self.presets
                .iter()
                .map(|preset| preset.label.clone())
                .collect()
        }
    }

    fn row_count(&self) -> usize {
        if self.showing_candidates() {
            self.candidates.len()
        } else {
            self.presets.len()
        }
    }

    /// Cycle focus Query → Name → List → Query. The list is skipped when it's
    /// empty, so Tab just toggles between the two input fields.
    pub(crate) fn switch_focus(&mut self) {
        let has_list = self.row_count() > 0;
        self.focus = match self.focus {
            EditLocationFocus::Query => EditLocationFocus::Name,
            EditLocationFocus::Name if has_list => EditLocationFocus::List,
            EditLocationFocus::Name => EditLocationFocus::Query,
            EditLocationFocus::List => EditLocationFocus::Query,
        };
    }

    /// Feed a key press to whichever text field has focus (inert on the list).
    /// Editing the query invalidates the last lookup.
    pub(crate) fn input_key(&mut self, key: crossterm::event::KeyEvent) {
        match self.focus {
            EditLocationFocus::Query => {
                if self.query.input(key) {
                    self.invalidate_lookup();
                }
            }
            EditLocationFocus::Name => {
                self.name.input(key);
            }
            EditLocationFocus::List => {}
        }
    }

    /// Editing the query invalidates the last lookup: drop the resolved result and
    /// candidate matches, clear the status preview, and flip Enter back to "look
    /// up". The typed name is untouched.
    fn invalidate_lookup(&mut self) {
        self.query_looked_up = false;
        self.resolved = None;
        self.candidates.clear();
        self.status = LocationResolveStatus::Idle;
        self.normalize_list_state();
    }

    /// Fold a finished forward-geocode reply into the dialog: replace the
    /// candidate list, move focus onto it when there are matches, and update the
    /// status line.
    pub(crate) fn apply_candidates(&mut self, hits: Vec<GeocodeHit>) {
        self.candidates = hits;
        self.list.set_offset(0);
        if self.candidates.is_empty() {
            self.status = LocationResolveStatus::NoMatch;
            // Don't leave focus stranded on a list that just emptied.
            if self.focus == EditLocationFocus::List && self.presets.is_empty() {
                self.focus = EditLocationFocus::Query;
            }
        } else {
            self.status = LocationResolveStatus::Resolved;
            self.focus = EditLocationFocus::List;
            self.select_index(0);
        }
        self.normalize_list_state();
    }

    /// Adopt a freshly grabbed device fix: mirror the coordinates into the query
    /// field and make them — with their accuracy and provider — the resolved,
    /// saveable value. Any stale address fields are dropped; the reverse-geocoded
    /// names for this new spot arrive next, via
    /// [`apply_reverse`](Self::apply_reverse).
    pub(crate) fn seed_device_fix(&mut self, fix: &DeviceFix) {
        self.query = TextInput::from(format!(
            "{}, {}",
            fix.coordinates.latitude(),
            fix.coordinates.longitude()
        ));
        let mut location = Location {
            accuracy_m: fix.accuracy_m,
            source: Some(fix.source.to_string()),
            ..Location::default()
        };
        location.set_coordinates(fix.coordinates);
        self.resolved = Some(location);
    }

    /// Fold a finished reverse-geocode reply into the dialog: enrich the resolved
    /// coordinates with the returned names (keeping the user's coordinates). The
    /// coordinates are now looked up, so Enter in the address field will save.
    pub(crate) fn apply_reverse(&mut self, hit: Option<GeocodeHit>) {
        match hit {
            Some(hit) => {
                let mut location = hit.location;
                // Keep the coordinates the user entered or the device grabbed,
                // along with that grab's accuracy and provider.
                if let Some(resolved) = &self.resolved {
                    location.latitude = resolved.latitude.or(location.latitude);
                    location.longitude = resolved.longitude.or(location.longitude);
                    location.accuracy_m = resolved.accuracy_m.or(location.accuracy_m);
                    location.source = resolved.source.clone().or(location.source);
                }
                // A POI/venue name fills the name field unless the user typed one,
                // so composed() (which takes the name from that field) keeps it.
                if self.name.as_str().trim().is_empty()
                    && let Some(name) = &location.name
                {
                    self.name = TextInput::from(name.clone());
                }
                self.resolved = Some(location);
                self.status = LocationResolveStatus::Resolved;
            }
            // The coordinates the user entered are still resolved and saveable;
            // only the name lookup came back empty.
            None => self.status = LocationResolveStatus::NoMatch,
        }
        self.query_looked_up = true;
    }

    /// Adopt the highlighted preset/candidate as the resolved location, seeding
    /// the query field from it. Its name (a preset's label, or a POI/venue name
    /// from geocoding) fills the name field only when the user hasn't typed one,
    /// so a deliberate custom name is never clobbered.
    pub(crate) fn select_row(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let location = if self.showing_candidates() {
            self.candidates.get(index).map(|hit| hit.location.clone())
        } else {
            self.presets
                .get(index)
                .map(|preset| preset.location.clone())
        };
        if let Some(location) = location {
            if self.name.as_str().trim().is_empty()
                && let Some(name) = &location.name
            {
                self.name = TextInput::from(name.clone());
            }
            self.query = TextInput::from(query_seed(&location));
            self.resolved = Some(location);
            self.status = LocationResolveStatus::Resolved;
        }
    }

    /// The location to persist: the resolved coordinates/address with the typed
    /// name applied. `None` when nothing is set (clears the entry's location).
    pub(crate) fn composed(&self) -> Option<Location> {
        let mut location = self.resolved.clone().unwrap_or_default();
        let name = self.name.as_str().trim();
        location.name = (!name.is_empty()).then(|| name.to_string());
        (!location.is_empty()).then_some(location)
    }
}

impl ListNav for EditLocationState {
    fn list(&self) -> &SelectableList {
        &self.list
    }

    fn list_mut(&mut self) -> &mut SelectableList {
        &mut self.list
    }

    fn item_count(&self) -> usize {
        self.row_count()
    }
}

/// Seed the address/coords field from a location: its coordinates when known (so
/// it stays re-resolvable). Empty otherwise — the place name lives in its own
/// field and must not be echoed here.
fn query_seed(location: &Location) -> String {
    match (location.latitude, location.longitude) {
        (Some(lat), Some(lon)) => format!("{lat}, {lon}"),
        _ => String::new(),
    }
}

/// Parse `"lat, lon"` into validated coordinates. `None` when it isn't two
/// comma-separated numbers within the valid latitude/longitude ranges — the
/// signal to treat the input as an address instead.
pub(crate) fn parse_coordinates(input: &str) -> Option<notema_domain::Coordinates> {
    let (lat, lon) = input.split_once(',')?;
    let lat: f64 = lat.trim().parse().ok()?;
    let lon: f64 = lon.trim().parse().ok()?;
    notema_domain::Coordinates::try_new(lat, lon).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_coordinates_accepts_valid_and_rejects_out_of_range() {
        assert_eq!(
            parse_coordinates("52.52, 13.405"),
            notema_domain::Coordinates::try_new(52.52, 13.405).ok()
        );
        assert_eq!(
            parse_coordinates("  -33.8, 151.2 "),
            notema_domain::Coordinates::try_new(-33.8, 151.2).ok()
        );
        // Out of range.
        assert_eq!(parse_coordinates("91, 0"), None);
        assert_eq!(parse_coordinates("0, 181"), None);
        // Not two numbers.
        assert_eq!(parse_coordinates("Berlin"), None);
        assert_eq!(parse_coordinates("52.52"), None);
        assert_eq!(parse_coordinates("a, b"), None);
    }

    fn hit(name: &str, lat: f64, lon: f64) -> GeocodeHit {
        GeocodeHit {
            display_name: name.to_string(),
            location: Location {
                city: Some(name.to_string()),
                latitude: Some(lat),
                longitude: Some(lon),
                ..Location::default()
            },
        }
    }

    fn device_fix(lat: f64, lon: f64) -> DeviceFix {
        DeviceFix {
            coordinates: notema_domain::Coordinates::try_new(lat, lon).unwrap(),
            accuracy_m: Some(12.0),
            source: notema_context::DeviceLocationSource::CoreLocation,
        }
    }

    #[test]
    fn location_composes_name_only_and_clears_when_empty() {
        let mut state = EditLocationState::new(None, Vec::new());
        assert_eq!(state.composed(), None);

        state.name = "Home".into();
        let composed = state.composed().unwrap();
        assert_eq!(composed.name.as_deref(), Some("Home"));
        assert!(composed.latitude.is_none());
    }

    #[test]
    fn location_selecting_a_candidate_fills_resolved_and_saves_coordinates() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.apply_candidates(vec![hit("Paris", 48.85, 2.35)]);

        // A match list takes focus and reports resolved.
        assert!(state.showing_candidates());
        assert_eq!(state.focus, EditLocationFocus::List);
        assert_eq!(state.status, LocationResolveStatus::Resolved);

        state.select_row();
        let composed = state.composed().unwrap();
        assert_eq!(composed.latitude, Some(48.85));
        assert_eq!(composed.city.as_deref(), Some("Paris"));
    }

    #[test]
    fn location_picking_a_geocoded_address_keeps_the_typed_name() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.name = "Home".into();
        // A plain address candidate carries a road/city but no POI name.
        let candidate = GeocodeHit {
            display_name: "Bahnhofstraße 1, Berlin".to_string(),
            location: Location {
                road: Some("Bahnhofstraße".to_string()),
                house_number: Some("1".to_string()),
                city: Some("Berlin".to_string()),
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Location::default()
            },
        };
        state.apply_candidates(vec![candidate]);
        state.select_row();

        let composed = state.composed().unwrap();
        assert_eq!(
            composed.name.as_deref(),
            Some("Home"),
            "typed name survives"
        );
        assert_eq!(composed.road.as_deref(), Some("Bahnhofstraße"));
        assert_eq!(composed.city.as_deref(), Some("Berlin"));
        assert_eq!(composed.latitude, Some(52.52));
    }

    #[test]
    fn location_picking_a_poi_fills_an_empty_name() {
        // No name typed: a candidate's POI name (a shop/venue) fills it.
        let mut state = EditLocationState::new(None, Vec::new());
        let candidate = GeocodeHit {
            display_name: "Corner Cafe, Bahnhofstraße 1, Berlin".to_string(),
            location: Location {
                name: Some("Corner Cafe".to_string()),
                road: Some("Bahnhofstraße".to_string()),
                house_number: Some("1".to_string()),
                city: Some("Berlin".to_string()),
                ..Location::default()
            },
        };
        state.apply_candidates(vec![candidate]);
        state.select_row();

        assert_eq!(
            state.composed().unwrap().name.as_deref(),
            Some("Corner Cafe")
        );
    }

    #[test]
    fn location_no_candidates_reports_no_match_and_keeps_presets() {
        let preset = LocationPreset {
            label: "Berlin".to_string(),
            location: Location {
                city: Some("Berlin".to_string()),
                ..Location::default()
            },
        };
        let mut state = EditLocationState::new(None, vec![preset]);
        state.apply_candidates(Vec::new());

        assert!(!state.showing_candidates());
        assert_eq!(state.status, LocationResolveStatus::NoMatch);
        assert_eq!(state.item_count(), 1, "presets stay listed");
    }

    #[test]
    fn location_tab_skips_the_list_when_empty() {
        // No presets and no candidates: Tab only toggles the two input fields.
        let mut state = EditLocationState::new(None, Vec::new());
        assert_eq!(state.focus, EditLocationFocus::Query);
        state.switch_focus();
        assert_eq!(state.focus, EditLocationFocus::Name);
        state.switch_focus();
        assert_eq!(state.focus, EditLocationFocus::Query, "the list is skipped");

        // With a preset present, the list joins the cycle.
        let preset = LocationPreset {
            label: "Berlin".to_string(),
            location: Location {
                city: Some("Berlin".to_string()),
                ..Location::default()
            },
        };
        let mut state = EditLocationState::new(None, vec![preset]);
        state.switch_focus(); // Query -> Name
        state.switch_focus(); // Name -> List
        assert_eq!(state.focus, EditLocationFocus::List);
    }

    #[test]
    fn location_reverse_keeps_user_coordinates_and_adds_names() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.resolved = Some(Location {
            latitude: Some(1.0),
            longitude: Some(2.0),
            ..Location::default()
        });
        state.apply_reverse(Some(hit("Town", 9.9, 9.9)));

        let resolved = state.resolved.unwrap();
        assert_eq!(resolved.latitude, Some(1.0));
        assert_eq!(resolved.longitude, Some(2.0));
        assert_eq!(resolved.city.as_deref(), Some("Town"));
    }

    #[test]
    fn location_reverse_poi_name_survives_into_composed() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.seed_device_fix(&device_fix(52.52, 13.405));
        // A reverse hit that carries a POI/venue name, user hasn't typed one.
        let poi = GeocodeHit {
            display_name: "Corner Cafe".to_string(),
            location: Location {
                name: Some("Corner Cafe".to_string()),
                city: Some("Berlin".to_string()),
                latitude: Some(9.9),
                longitude: Some(9.9),
                ..Location::default()
            },
        };
        state.apply_reverse(Some(poi));

        let composed = state.composed().unwrap();
        assert_eq!(
            composed.name.as_deref(),
            Some("Corner Cafe"),
            "POI name saved"
        );
        assert_eq!(composed.latitude, Some(52.52), "grabbed coordinates kept");
    }

    #[test]
    fn location_device_grab_seeds_coords_then_reverse_names_them() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.name = "Desk".into();
        // Simulate a stale prior address to prove the grab starts clean.
        state.resolved = Some(Location {
            city: Some("Elsewhere".to_string()),
            ..Location::default()
        });

        // A grabbed fix mirrors into the query field and becomes the resolved,
        // saveable coordinates (with accuracy + provider) — stale address dropped.
        state.seed_device_fix(&device_fix(52.52, 13.405));
        assert_eq!(state.query.as_str(), "52.52, 13.405");
        let resolved = state.resolved.clone().unwrap();
        assert_eq!(resolved.latitude, Some(52.52));
        assert_eq!(resolved.longitude, Some(13.405));
        assert_eq!(resolved.accuracy_m, Some(12.0));
        assert_eq!(resolved.source.as_deref(), Some("corelocation"));
        assert!(resolved.city.is_none(), "stale address is cleared");

        // The reverse lookup then names the spot, keeping the grabbed coordinates
        // (with accuracy + provider) and the name the user had typed.
        state.apply_reverse(Some(hit("Berlin", 9.9, 9.9)));
        let composed = state.composed().unwrap();
        assert_eq!(composed.latitude, Some(52.52));
        assert_eq!(composed.longitude, Some(13.405));
        assert_eq!(composed.accuracy_m, Some(12.0), "device accuracy kept");
        assert_eq!(
            composed.source.as_deref(),
            Some("corelocation"),
            "provider kept"
        );
        assert_eq!(composed.city.as_deref(), Some("Berlin"));
        assert_eq!(composed.name.as_deref(), Some("Desk"), "typed name kept");
    }

    #[test]
    fn location_opened_with_coords_only_still_needs_a_lookup() {
        // Only coordinates stored: Enter should look up, not save.
        let coords_only = Location {
            latitude: Some(52.5),
            longitude: Some(13.4),
            ..Location::default()
        };
        let state = EditLocationState::new(Some(coords_only), Vec::new());
        assert!(!state.query.is_empty());
        assert!(!state.query_looked_up);

        // Coordinates plus address detail count as already resolved.
        let resolved = Location {
            city: Some("Berlin".to_string()),
            latitude: Some(52.5),
            longitude: Some(13.4),
            ..Location::default()
        };
        let state = EditLocationState::new(Some(resolved), Vec::new());
        assert!(state.query_looked_up);
    }

    #[test]
    fn location_query_flips_to_save_after_lookup_then_back_on_edit() {
        let mut state = EditLocationState::new(None, Vec::new());
        state.focus = EditLocationFocus::Query;
        state.query = "52.5, 13.4".into();
        state.resolved = Some(Location {
            latitude: Some(52.5),
            longitude: Some(13.4),
            ..Location::default()
        });

        // A finished reverse lookup marks the query resolved (Enter would save).
        state.apply_reverse(Some(hit("Berlin", 52.5, 13.4)));
        assert!(state.query_looked_up);
        assert!(state.resolved.is_some());

        // Editing the query reverts to look-up mode and clears the shown result.
        state.input_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('5'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(!state.query_looked_up);
        assert!(state.resolved.is_none());
        assert_eq!(state.status, LocationResolveStatus::Idle);
    }
}
