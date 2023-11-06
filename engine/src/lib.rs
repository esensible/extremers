#![cfg_attr(not(test), no_std)]

pub mod callbacks;
mod event_core;
mod flatdiff;
mod serde_engine;
use serde::Serialize;

pub use crate::flatdiff::{Flat, FlatDiff, FlatDiffSer, Atomic};
pub use flatdiff_derive::FlatDiffSer;

pub use crate::event_core::SleepFn;
pub use crate::event_core::{EngineCore, EventEngine, EventEngineTrait};
pub use crate::serde_engine::SerdeEngine;
pub use callbacks as engine_callbacks;

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
