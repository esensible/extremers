use ::serde::{Deserialize, Serialize};

use crate::core::Atomic;

#[derive(Serialize, Deserialize, Copy, Clone, PartialEq, Default)]
pub struct Location {
    lat: f64,
    lon: f64,
}
impl Atomic for Location {}
