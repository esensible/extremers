//! Race: the start sequence, setting the line, and the time and place the
//! boat would cross it. An engine package (`extreme_sdk`): natively a
//! library (tested on the host), as WebAssembly the package's engine.wasm.

#![no_std]

mod geo_math;
mod line;
mod race;
mod types;
pub use race::{LineView, Phase, Race, RaceEvent, RaceView};

extreme_sdk::export_engine!(crate::Race);

#[cfg(test)]
mod race_tests;
