#![no_std]

// First, so its logging macros are visible to every module below.
#[macro_use]
mod fmt;

#[cfg(feature = "ble")]
pub mod ble;
pub mod ble_settings;
pub mod bulk;
pub mod config;
pub mod engine;
pub mod host;
pub mod http;
pub mod nmea;
pub mod tasks;

/// Memory use on the device, for the logs (see the targets' heap/stack
/// logging): how much heap wasm3 holds, where the target can tell.
pub use extreme_wasm3 as wasm3;
