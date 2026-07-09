use super::App;
use chrono::{DateTime, FixedOffset};
use journal_context_provider::compute_celestial;
use journal_core::{Location, MetadataField};
use std::path::Path;

impl App {
    /// Capture environment data — weather, celestial, air quality — for an entry
    /// once its location is known. Celestial is computed locally, so it's written
    /// immediately (even offline); weather and air quality need the network and are
    /// fetched in the background, then persisted by
    /// [`apply_environment_results`](Self::apply_environment_results). A no-op when
    /// the location has no coordinates. Persistence errors are dropped so a failed
    /// lookup never disturbs the entry save that triggered it.
    pub(crate) fn capture_environment_for_entry(
        &mut self,
        path: &Path,
        location: &Location,
        datetime: DateTime<FixedOffset>,
    ) {
        let (Some(lat), Some(lon)) = (location.latitude, location.longitude) else {
            return;
        };

        let celestial = compute_celestial(lat, lon, datetime);
        let _ = self
            .store
            .set_entry_metadata_field(path, MetadataField::Celestial(Some(Box::new(celestial))));

        self.environment
            .request(path.to_path_buf(), lat, lon, datetime);
    }

    /// Clear an entry's captured environment data — weather, celestial, and air
    /// quality — used when its location is removed.
    pub(crate) fn clear_environment_for_entry(&mut self, path: &Path) {
        self.environment.forget(path);
        let _ = self.store.set_entry_metadata_fields(
            path,
            &[
                MetadataField::Weather(None),
                MetadataField::Celestial(None),
                MetadataField::AirQuality(None),
            ],
        );
    }

    /// Persist any finished environment lookups to their entries. Weather and air
    /// quality arrive from one lookup, so they land in a single write. The file
    /// lands on disk and the watcher picks it up; nothing is shown in the UI, so
    /// this doesn't itself request a repaint.
    pub(crate) fn apply_environment_results(&mut self) {
        for (path, weather, air_quality) in self.environment.drain() {
            let mut fields = Vec::new();
            if let Some(weather) = weather {
                fields.push(MetadataField::Weather(Some(Box::new(weather))));
            }
            if let Some(air_quality) = air_quality {
                fields.push(MetadataField::AirQuality(Some(Box::new(air_quality))));
            }
            let _ = self.store.set_entry_metadata_fields(&path, &fields);
        }
    }
}
