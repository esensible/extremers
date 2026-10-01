use core::f64::consts::PI;

use extreme_traits::Fix;

/// A position in radians.
#[derive(Copy, Clone, PartialEq, Default, Debug)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}

impl From<Fix> for Location {
    /// Degrees to radians. Deliberately `x * PI / 180.0` rather than
    /// `f64::to_radians` (which computes `x * (PI / 180.0)` and can differ in
    /// the last bit).
    fn from(fix: Fix) -> Self {
        Location {
            lat: fix.lat * PI / 180.0,
            lon: fix.lon * PI / 180.0,
        }
    }
}
