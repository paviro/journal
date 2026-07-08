use super::App;
use chrono::{DateTime, FixedOffset};
use journal_storage::{Location, MetadataField, compute_celestial};
use std::path::Path;

impl App {
    /// Capture weather and celestial data for an entry once its location is
    /// known. Celestial is computed locally, so it's written immediately (even
    /// offline); weather needs the network and is fetched in the background, then
    /// persisted by [`apply_weather_results`](Self::apply_weather_results). A
    /// no-op when the location has no coordinates. Persistence errors are dropped
    /// so a failed lookup never disturbs the entry save that triggered it.
    pub(crate) fn capture_weather_for_entry(
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

        self.weather.request(path.to_path_buf(), lat, lon, datetime);
    }

    /// Clear an entry's captured weather and celestial data — used when its
    /// location is removed.
    pub(crate) fn clear_weather_for_entry(&mut self, path: &Path) {
        self.weather.forget(path);
        let _ = self
            .store
            .set_entry_metadata_field(path, MetadataField::Weather(None));
        let _ = self
            .store
            .set_entry_metadata_field(path, MetadataField::Celestial(None));
    }

    /// Persist any finished weather lookups to their entries. The write lands on
    /// disk and the file watcher picks it up; nothing is shown in the UI, so this
    /// doesn't itself request a repaint.
    pub(crate) fn apply_weather_results(&mut self) {
        for (path, weather) in self.weather.drain() {
            let _ = self
                .store
                .set_entry_metadata_field(&path, MetadataField::Weather(Some(Box::new(weather))));
        }
    }
}
