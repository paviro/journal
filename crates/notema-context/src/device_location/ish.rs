//! iSH provider (iOS): iSH exposes a live GPS stream at `/dev/location` that
//! prints one `+lat,+lon` line per reading, e.g. `+48.096581,+11.528263`. We
//! open it, take the first parsable line, and stop.

use super::{DeviceFix, DeviceLocationSource};
use crate::{ContextError, Result};
use notema_domain::Coordinates;
use std::{
    fs::File,
    io::Read,
    thread,
    time::{Duration, Instant},
};

/// How long to wait for a reading. The stream emits continuously once Location
/// is authorised, so this only has to cover the "permission off / never emits"
/// case; the dialog shows "Resolving…" meanwhile.
const TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn locate() -> Result<DeviceFix> {
    let file = match File::open("/dev/location") {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ContextError::message(
                "iSH location device (/dev/location) not found — update iSH",
            ));
        }
        Err(error) => return Err(error.into()),
    };

    // Read the device directly in nonblocking mode. iSH's character device does
    // not report readiness through poll(2), even while a plain `cat` receives
    // fixes, and a detached blocking reader would leak on every timeout.
    let flags = rustix::fs::fcntl_getfl(&file).map_err(|error| {
        ContextError::message(format!("could not inspect /dev/location: {error}"))
    })?;
    rustix::fs::fcntl_setfl(&file, flags | rustix::fs::OFlags::NONBLOCK).map_err(|error| {
        ContextError::message(format!("could not configure /dev/location: {error}"))
    })?;
    let mut file = file;
    let deadline = Instant::now() + TIMEOUT;
    let mut pending = String::new();
    let mut chunk = [0_u8; 256];
    let coordinates = 'reading: loop {
        if Instant::now() >= deadline {
            break None;
        }

        match file.read(&mut chunk) {
            Ok(0) => thread::sleep(Duration::from_millis(25)),
            Ok(read) => {
                pending.push_str(&String::from_utf8_lossy(&chunk[..read]));
                while let Some(newline) = pending.find('\n') {
                    let line = pending[..newline].to_string();
                    pending.drain(..=newline);
                    if let Some(coordinates) = parse_line(&line) {
                        break 'reading Some(coordinates);
                    }
                }
                if pending.len() > 4096 {
                    pending.clear();
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    };

    match coordinates {
        Some(coordinates) => Ok(DeviceFix {
            coordinates,
            accuracy_m: None,
            source: DeviceLocationSource::Ish,
        }),
        None => Err(ContextError::message(
            "no location fix from iSH — enable Location access for iSH in iOS Settings › Privacy",
        )),
    }
}

/// Parse one `+lat,+lon` line into coordinates. Returns `None` for blank or
/// malformed lines so the reader can skip past them to the next reading.
fn parse_line(line: &str) -> Option<Coordinates> {
    let (lat, lon) = line.trim().split_once(',')?;
    // `f64::from_str` accepts the leading `+`.
    let latitude = lat.trim().parse().ok()?;
    let longitude = lon.trim().parse().ok()?;
    Coordinates::try_new(latitude, longitude).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_reads_signed_coordinates() {
        let coordinates = parse_line("+48.096581,+11.528263").unwrap();
        assert_eq!(coordinates.latitude(), 48.096581);
        assert_eq!(coordinates.longitude(), 11.528263);
    }

    #[test]
    fn parse_line_rejects_junk() {
        assert!(parse_line("").is_none());
        assert!(parse_line("not,coordinates").is_none());
        assert!(parse_line("+48.0").is_none());
        // Out of range → Coordinates::try_new rejects it.
        assert!(parse_line("+200.0,+11.0").is_none());
    }
}
