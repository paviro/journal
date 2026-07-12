//! The `notema licenses` command: the app's own license and the required
//! attributions for the external data sources it queries, followed by the
//! third-party dependency license report embedded at build time (see build.rs).

use crate::AppResult;
use anyhow::{Context, bail};
use flate2::read::GzDecoder;
use std::{collections::BTreeMap, io::Read};

/// The gzipped JSON report `build.rs` writes to `OUT_DIR`. When license
/// generation is skipped this is a gzipped `[]`, so the dependency listing is
/// empty but the data-source attributions above it still print.
const LICENSES_GZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/LICENSES.json.gz"));

#[derive(serde::Deserialize)]
struct LicenseGroup {
    license: String,
    text: String,
    dependencies: Vec<Dependency>,
}

#[derive(serde::Deserialize)]
struct Dependency {
    name: String,
    version: String,
}

/// Print attributions and third-party licenses. With `dependency`, print the
/// full license text for the matching crate instead of the grouped summary.
pub(crate) fn run(dependency: Option<String>) -> AppResult<()> {
    if dependency.is_none() {
        print_data_sources();
    }
    print_dependencies(dependency)
}

/// The app's own license plus the credits the data providers require: Open-Meteo
/// (CC BY 4.0) for weather and air quality, and OpenStreetMap/Nominatim (ODbL)
/// for geocoding. Printed whenever the whole report is shown.
fn print_data_sources() {
    println!("notema {} — EUPL-1.2", env!("CARGO_PKG_VERSION"));
    println!("https://github.com/paviro/notema");
    println!();
    println!("Data sources:");
    println!("  Weather & air quality data from Open-Meteo (CC BY 4.0)");
    println!("    https://open-meteo.com");
    println!("  Location geocoding: © OpenStreetMap contributors, via Nominatim (ODbL)");
    println!("    https://www.openstreetmap.org/copyright");
    println!();
}

fn embedded_groups() -> AppResult<Vec<LicenseGroup>> {
    let mut json = String::new();
    GzDecoder::new(LICENSES_GZ)
        .read_to_string(&mut json)
        .context("failed to decompress embedded license data")?;
    serde_json::from_str(&json).context("failed to parse embedded license data")
}

fn print_dependencies(dependency: Option<String>) -> AppResult<()> {
    let groups = embedded_groups()?;
    match dependency {
        None => print_summary(&groups),
        Some(query) => print_one(&groups, &query)?,
    }
    Ok(())
}

/// List every crate grouped by SPDX license identifier.
fn print_summary(groups: &[LicenseGroup]) {
    if groups.is_empty() {
        println!(
            "No third-party license data embedded (built with NOTEMA_SKIP_LICENSE_GENERATION)."
        );
        return;
    }

    let mut by_spdx: BTreeMap<&str, Vec<&Dependency>> = BTreeMap::new();
    for group in groups {
        by_spdx
            .entry(group.license.as_str())
            .or_default()
            .extend(&group.dependencies);
    }

    for (spdx, deps) in &mut by_spdx {
        deps.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
        let count = deps.len();
        println!(
            "{spdx} ({count} {})",
            if count == 1 {
                "dependency"
            } else {
                "dependencies"
            }
        );
        for dep in deps {
            println!("  {} {}", dep.name, dep.version);
        }
        println!();
    }
    println!("Use 'notema licenses <dependency>' to view the full license text.");
}

/// Show the full license text for the crate matching `query`, preferring an
/// exact name match and falling back to a substring search.
fn print_one(groups: &[LicenseGroup], query: &str) -> AppResult<()> {
    let query = query.to_lowercase();
    let mut matches: Vec<(&Dependency, &LicenseGroup)> = groups
        .iter()
        .flat_map(|group| group.dependencies.iter().map(move |dep| (dep, group)))
        .filter(|(dep, _)| dep.name.to_lowercase() == query)
        .collect();
    if matches.is_empty() {
        matches = groups
            .iter()
            .flat_map(|group| group.dependencies.iter().map(move |dep| (dep, group)))
            .filter(|(dep, _)| dep.name.to_lowercase().contains(&query))
            .collect();
    }

    match matches.as_slice() {
        [] => bail!("no dependency found matching '{query}'"),
        [(dep, group)] => {
            println!("{} {} — {}\n", dep.name, dep.version, group.license);
            println!("{}", group.text);
        }
        _ => {
            println!("Multiple dependencies match '{query}':\n");
            for (dep, group) in &matches {
                println!("  {} {} ({})", dep.name, dep.version, group.license);
            }
            println!("\nSpecify the exact dependency name.");
        }
    }
    Ok(())
}
