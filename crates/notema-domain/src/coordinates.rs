use serde::{Deserialize, Deserializer, Serialize, de};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Coordinates {
    latitude: f64,
    longitude: f64,
}

impl Coordinates {
    pub fn try_new(latitude: f64, longitude: f64) -> Result<Self, CoordinateError> {
        if latitude.is_finite()
            && longitude.is_finite()
            && (-90.0..=90.0).contains(&latitude)
            && (-180.0..=180.0).contains(&longitude)
        {
            Ok(Self {
                latitude,
                longitude,
            })
        } else {
            Err(CoordinateError {
                latitude,
                longitude,
            })
        }
    }

    pub fn latitude(self) -> f64 {
        self.latitude
    }

    pub fn longitude(self) -> f64 {
        self.longitude
    }

    /// Parse `"lat, lon"` into validated coordinates. `None` when it isn't two
    /// comma-separated numbers within the valid latitude/longitude ranges — the
    /// signal to treat the input as an address instead.
    pub fn parse(input: &str) -> Option<Self> {
        let (lat, lon) = input.split_once(',')?;
        let lat: f64 = lat.trim().parse().ok()?;
        let lon: f64 = lon.trim().parse().ok()?;
        Self::try_new(lat, lon).ok()
    }
}

impl<'de> Deserialize<'de> for Coordinates {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            latitude: f64,
            longitude: f64,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::try_new(raw.latitude, raw.longitude).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoordinateError {
    pub latitude: f64,
    pub longitude: f64,
}

impl fmt::Display for CoordinateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid coordinates ({}, {}): latitude must be -90..=90 and longitude -180..=180",
            self.latitude, self.longitude
        )
    }
}

impl std::error::Error for CoordinateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_boundary_coordinates() {
        assert!(Coordinates::try_new(-90.0, -180.0).is_ok());
        assert!(Coordinates::try_new(90.0, 180.0).is_ok());
    }

    #[test]
    fn rejects_non_finite_and_out_of_range_values() {
        assert!(Coordinates::try_new(f64::NAN, 0.0).is_err());
        assert!(Coordinates::try_new(90.1, 0.0).is_err());
        assert!(Coordinates::try_new(0.0, -180.1).is_err());
    }

    #[test]
    fn parse_accepts_valid_and_rejects_out_of_range() {
        assert_eq!(
            Coordinates::parse("52.52, 13.405"),
            Coordinates::try_new(52.52, 13.405).ok()
        );
        assert_eq!(
            Coordinates::parse("  -33.8, 151.2 "),
            Coordinates::try_new(-33.8, 151.2).ok()
        );
        // Out of range.
        assert_eq!(Coordinates::parse("91, 0"), None);
        assert_eq!(Coordinates::parse("0, 181"), None);
        // Not two numbers.
        assert_eq!(Coordinates::parse("Berlin"), None);
        assert_eq!(Coordinates::parse("52.52"), None);
        assert_eq!(Coordinates::parse("a, b"), None);
    }
}
