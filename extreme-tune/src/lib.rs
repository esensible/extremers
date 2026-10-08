//! Tune: speed, and how speed and heading differ from their 30 s average.
//! An engine package (`extreme_sdk`): natively a library (tested on the
//! host), as WebAssembly the package's engine.wasm.

#![no_std]

mod tune;
pub use tune::{TuneSpeed, TuneView};

/// The engine the package runs: 32 samples of history (more than the 30 s
/// window at the GPS's 1 Hz).
pub type Tune = TuneSpeed<32>;

extreme_sdk::export_engine!(crate::Tune);

#[cfg(test)]
mod tune_tests;
