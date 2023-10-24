use ::serde::{Deserialize, Serialize};

use crate::core::Atomic;

#[derive(Serialize, Deserialize, Copy, Clone, PartialEq, Default)]
pub struct Location {
    pub lat: f32,
    pub lon: f32,
}
impl Atomic for Location {}
