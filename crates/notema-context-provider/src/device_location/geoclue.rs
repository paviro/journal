//! Linux provider: GeoClue2 over the system D-Bus. Note that GeoClue's default
//! Wi-Fi backend (Mozilla Location Service) was retired in 2024, so on a machine
//! without a GPS device and without a reconfigured backend (e.g. BeaconDB) this
//! returns "no location" — which the dialog surfaces plainly.

use super::{DeviceFix, DeviceLocationSource};
use crate::AppResult;
use std::time::Duration;
use zbus::{blocking::Connection, proxy, zvariant::OwnedObjectPath};

/// How long to wait for GeoClue to produce a fix before giving up.
const TIMEOUT: Duration = Duration::from_secs(30);
/// GeoClue's stable identifier for us; also the accuracy level we ask for.
const DESKTOP_ID: &str = "de.paviro.notema";
/// `GCLUE_ACCURACY_LEVEL_EXACT` — ask for the most precise fix available.
const ACCURACY_EXACT: u32 = 8;

#[proxy(
    interface = "org.freedesktop.GeoClue2.Manager",
    default_service = "org.freedesktop.GeoClue2",
    default_path = "/org/freedesktop/GeoClue2/Manager"
)]
trait Manager {
    fn get_client(&self) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.GeoClue2.Client",
    default_service = "org.freedesktop.GeoClue2"
)]
trait Client {
    fn start(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn set_desktop_id(&self, id: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn set_requested_accuracy_level(&self, level: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn location_updated(&self, old: OwnedObjectPath, new: OwnedObjectPath) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.GeoClue2.Location",
    default_service = "org.freedesktop.GeoClue2"
)]
trait Location {
    #[zbus(property)]
    fn latitude(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn longitude(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn accuracy(&self) -> zbus::Result<f64>;
}

pub(super) fn locate() -> AppResult<DeviceFix> {
    // The D-Bus exchange is blocking and may never answer if no backend can, so
    // bound it — nothing to clean up when it overruns.
    super::run_with_timeout(TIMEOUT, query_geoclue).unwrap_or_else(|| {
        Err(anyhow::anyhow!(
            "timed out waiting for a location fix from GeoClue"
        ))
    })
}

fn query_geoclue() -> AppResult<DeviceFix> {
    let connection = Connection::system()
        .map_err(|error| anyhow::anyhow!("cannot reach the system D-Bus: {error}"))?;

    let manager = ManagerProxyBlocking::new(&connection)
        .map_err(|_| anyhow::anyhow!("GeoClue2 is not available on this system"))?;
    let client_path = manager
        .get_client()
        .map_err(|error| anyhow::anyhow!("GeoClue refused a client: {error}"))?;

    let client = ClientProxyBlocking::builder(&connection)
        .path(client_path)?
        .build()?;
    client.set_desktop_id(DESKTOP_ID)?;
    client.set_requested_accuracy_level(ACCURACY_EXACT)?;

    // Subscribe before Start so we can't miss the first update.
    let mut updates = client.receive_location_updated()?;
    client
        .start()
        .map_err(|error| anyhow::anyhow!("GeoClue could not start locating: {error}"))?;

    let fix = match updates.next() {
        Some(signal) => {
            let args = signal.args()?;
            read_location(&connection, args.new)
        }
        None => Err(anyhow::anyhow!("GeoClue returned no location")),
    };
    let _ = client.stop();
    fix
}

fn read_location(connection: &Connection, path: OwnedObjectPath) -> AppResult<DeviceFix> {
    let location = LocationProxyBlocking::builder(connection)
        .path(path)?
        .build()?;
    Ok(DeviceFix {
        latitude: location.latitude()?,
        longitude: location.longitude()?,
        accuracy_m: location.accuracy().ok(),
        source: DeviceLocationSource::GeoClue,
    })
}
