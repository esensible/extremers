#![cfg_attr(not(test), no_std)]

mod race;
pub use race::Race;

mod core;
mod geo_math;
mod line;
mod types;
use serde::Serialize;

pub use crate::core::SleepFn;
pub use crate::core::{EventEngine, EventEngineTrait};
pub use crate::core::{Flat, FlatDiff};
pub use crate::core::{SerdeEngine, SerdeEngineTrait};

pub type RaceEngine = EventEngine<Race, 1>;

#[derive(Serialize)]
pub struct UpdateResp<T: Serialize> {
    cnt: usize,
    update: T,
}

impl<T: Serialize> UpdateResp<T> {
    pub fn new(cnt: usize, update: T) -> Self {
        Self { cnt, update }
    }
}
