#![no_std]

mod race;
pub use race::Race;
mod core;
// use core::RequestWrapper;
pub use crate::core::EventEngine;
mod line;
mod types;
use serde::Serialize;

pub type RaceEngine = EventEngine<Race, 1>;
pub use crate::core::EventEngineTrait;
pub use crate::core::SerdeEngine;
pub use crate::core::SerdeEngineTrait;

pub use crate::core::{Flat, FlatDiff};

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
