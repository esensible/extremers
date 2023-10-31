use ::serde::{Deserialize, Serialize};

use crate::core::Atomic;

#[derive(Serialize, Deserialize, Copy, Clone, PartialEq, Default)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}
impl Atomic for Location {}
