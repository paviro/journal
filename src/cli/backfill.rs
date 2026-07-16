//! `notema backfill`: fill in missing location names, weather, air quality, and
//! celestial data for existing located entries. This is the manual, on-demand
//! replacement for the automatic sweeps the TUI used to run — Nominatim's usage
//! policy discourages automated bulk querying, so enrichment of old entries now
//! only happens when the user asks for it. New entries still capture their
//! environment live when written.

use std::{thread, time::Duration};

use chrono::Local;
use notema_context::{compute_celestial, fetch_environment, reverse_geocode};
use notema_domain::{EntryEncryptionState, MetadataField};

use super::{Cli, unlock_if_encrypted};
use crate::{AppResult, startup};

/// Pause after any entry that hit the network, so consecutive lookups stay at or
/// under the providers' one-request-per-second ceiling (each entry makes at most
/// one Nominatim call — the strict one — before this gap).
const REQUEST_GAP: Duration = Duration::from_secs(1);

pub(super) fn run(cli: &Cli) -> AppResult<()> {
    let startup::Startup { mut store, .. } = startup::load_existing(cli.config.as_deref())?;
    unlock_if_encrypted(&mut store)?;

    let entries = store.scan_entries()?;
    let mut updated = 0usize;
    let mut complete = 0usize;
    let mut unlocated = 0usize;

    for entry in &entries {
        // Only entries we can rewrite (plaintext, or encrypted and unlocked).
        if !matches!(
            entry.encryption_state,
            EntryEncryptionState::Plain | EntryEncryptionState::EncryptedUnlocked
        ) {
            continue;
        }
        let Some(coordinates) = entry.location.as_ref().and_then(|loc| loc.coordinates()) else {
            unlocated += 1;
            continue;
        };
        let location = entry
            .location
            .as_ref()
            .expect("coordinates imply a location");
        let datetime = entry
            .created_time()
            .unwrap_or_else(|| Local::now().fixed_offset());

        let mut fields: Vec<MetadataField> = Vec::new();
        let mut hit_network = false;

        // Address names, when the entry has only bare coordinates.
        if !location.has_named_parts() {
            hit_network = true;
            match reverse_geocode(coordinates) {
                Ok(Some(hit)) => fields.push(MetadataField::Location(Some(Box::new(
                    hit.location.with_pin_from(location),
                )))),
                // Nominatim knows no name there (e.g. open sea) — leave the coordinates.
                Ok(None) => {}
                Err(error) => {
                    eprintln!(
                        "note: reverse geocoding {} failed ({error})",
                        entry.path.display()
                    );
                }
            }
        }

        // Weather and air quality come from the network; celestial is computed
        // locally. Fetch only what the entry is missing, and never overwrite what's
        // already there.
        let need_weather = entry.weather.is_none();
        let need_air = entry.air_quality.is_none();
        let need_celestial = entry.celestial.is_none();
        if need_weather || need_air {
            hit_network = true;
            let report = fetch_environment(coordinates, datetime);
            for warning in &report.warnings {
                eprintln!("note: {} — {}", entry.path.display(), warning.message);
            }
            if need_celestial {
                fields.push(MetadataField::Celestial(Some(Box::new(report.celestial))));
            }
            if need_weather && let Some(weather) = report.weather {
                fields.push(MetadataField::Weather(Some(Box::new(weather))));
            }
            if need_air && let Some(air_quality) = report.air_quality {
                fields.push(MetadataField::AirQuality(Some(Box::new(air_quality))));
            }
        } else if need_celestial {
            fields.push(MetadataField::Celestial(Some(Box::new(compute_celestial(
                coordinates,
                datetime,
            )))));
        }

        if fields.is_empty() {
            complete += 1;
        } else {
            store.set_entry_metadata_fields_quiet(&entry.path, &fields)?;
            updated += 1;
            println!("filled {}", entry.path.display());
        }

        if hit_network {
            thread::sleep(REQUEST_GAP);
        }
    }

    println!(
        "Backfill complete: {updated} updated, {complete} already complete, \
         {unlocated} without coordinates."
    );
    Ok(())
}
